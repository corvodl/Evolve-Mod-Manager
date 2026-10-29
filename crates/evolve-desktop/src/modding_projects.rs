//! Read-only project dashboard compatible with the private Qt manager's
//! mod-projects.json registry and .evolve-pak-workspace.json manifests.
//! This is a navigation/index, not a bypass for verified build preflight.
#[path = "png_dds_import.rs"]
mod png_dds_import;

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const MAX_PROJECTS: usize = 4096;
const MAX_DIRECTORIES: usize = 32768;
const MAX_ENTRIES: usize = 60000;
const MAX_HASHED_PER_WORKSPACE: u64 = 256 * 1024 * 1024;
const MAX_XML_BYTES: usize = 48 * 1024 * 1024;

/// Progress for copying verified imported assets into a private project.
/// Phase 0 = validating, 1 = copying, 2 = publishing, 3 = opening editor.
#[derive(Debug, Default)]
pub struct ProjectCopyProgress {
    pub phase: AtomicUsize,
    pub files_done: AtomicUsize,
    pub files_total: AtomicUsize,
    pub bytes_done: AtomicU64,
    pub bytes_total: AtomicU64,
}

/// Hash the bytes while copying, rather than re-reading each copied file.
fn copy_verified_member(
    source: &Path,
    destination: &Path,
    progress: Option<&ProjectCopyProgress>,
) -> Result<String, String> {
    let mut input = File::open(source).map_err(|e| e.to_string())?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let read = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|e| e.to_string())?;
        hasher.update(&buffer[..read]);
        if let Some(progress) = progress {
            progress
                .bytes_done
                .fetch_add(read as u64, Ordering::Relaxed);
        }
    }
    output.flush().map_err(|e| e.to_string())?;
    Ok(format!("{:x}", hasher.finalize()))
}

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
    pub enabled: bool,
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

/// Only explicitly registered paths may become new signed PAK members.
/// Existing manifest records remain the sole authority for replacements.
fn additions(manifest: &Value, originals: &[Value]) -> Result<Vec<String>, String> {
    let Some(items) = manifest.get("add_members") else {
        return Ok(Vec::new());
    };
    let items = items.as_array().ok_or("Invalid added-member list")?;
    if items.len() > 64 {
        return Err("More than 64 added members in one project".into());
    }
    let existing: HashSet<_> = originals
        .iter()
        .filter_map(|row| row["path"].as_str())
        .map(str::to_ascii_lowercase)
        .collect();
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for value in items {
        let path = value.as_str().ok_or("Added member name is not text")?;
        if !safe_member(path)
            || !evolve_core::archive::safe_member_name(path).is_ok_and(|safe| safe == path)
            || existing.contains(&path.to_ascii_lowercase())
            || !seen.insert(path.to_ascii_lowercase())
        {
            return Err(format!(
                "Unsafe, duplicate, or existing added member: {path}"
            ));
        }
        result.push(path.to_owned());
    }
    Ok(result)
}

/// Register a user-owned file in an editable project; never infer additions
/// by scanning unrelated workspace files. Build Mod revalidates everything.
pub fn register_added_member(
    projects: &Path,
    project: &ProjectRow,
    member: &str,
) -> Result<(), String> {
    let root = fs::canonicalize(projects).map_err(|e| e.to_string())?;
    let folder = fs::canonicalize(&project.workspace).map_err(|e| e.to_string())?;
    if !folder.starts_with(&root)
        || folder == root
        || !safe_member(member)
        || !evolve_core::archive::safe_member_name(member).is_ok_and(|safe| safe == member)
    {
        return Err("Choose a safe relative member path in the selected project".into());
    }
    let manifest_path = folder.join(".evolve-pak-workspace.json");
    let mut manifest = read_json(&manifest_path, 8 * 1024 * 1024)?;
    if manifest["version"].as_u64() != Some(1) || manifest["reference_only"].as_bool() == Some(true)
    {
        return Err("Project is not an editable signed-PAK workspace".into());
    }
    let originals = manifest["entries"]
        .as_array()
        .ok_or("Missing original member index")?;
    let mut list = additions(&manifest, originals)?;
    if list.len() >= 64
        || list.iter().any(|value| value.eq_ignore_ascii_case(member))
        || originals.iter().any(|row| {
            row["path"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(member))
        })
    {
        return Err("Member already exists or addition limit reached".into());
    }
    let files = fs::canonicalize(folder.join("files")).map_err(|e| e.to_string())?;
    let source = folder.join("files").join(member);
    if !regular(&source, 16 * 1024 * 1024 * 1024)
        || !fs::canonicalize(&source)
            .map_err(|e| e.to_string())?
            .starts_with(&files)
    {
        return Err("Place a regular new file under this project's files/ directory first".into());
    }
    list.push(member.to_owned());
    manifest["add_members"] = serde_json::to_value(list).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(&folder).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut temp, &manifest).map_err(|e| e.to_string())?;
    temp.flush().map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(&manifest_path)
        .map_err(|e| e.error.to_string())?;
    Ok(())
}

/// Remove only an explicit registration; retain the user's file and all
/// original project records. A later build again becomes replacement-only
/// when this was the last addition.
pub fn unregister_added_member(
    projects: &Path,
    project: &ProjectRow,
    member: &str,
) -> Result<(), String> {
    let root = fs::canonicalize(projects).map_err(|e| e.to_string())?;
    let folder = fs::canonicalize(&project.workspace).map_err(|e| e.to_string())?;
    if folder == root || !folder.starts_with(&root) || !safe_member(member) {
        return Err("Choose an exact registered member in the selected project".into());
    }
    let manifest_path = folder.join(".evolve-pak-workspace.json");
    let mut manifest = read_json(&manifest_path, 8 * 1024 * 1024)?;
    if manifest["version"].as_u64() != Some(1) || manifest["reference_only"].as_bool() == Some(true)
    {
        return Err("Project is not an editable signed-PAK workspace".into());
    }
    let entries = manifest["entries"]
        .as_array()
        .ok_or("Missing original member index")?;
    let mut list = additions(&manifest, entries)?;
    let count = list.len();
    list.retain(|path| path != member);
    if count == list.len() {
        return Err("That path is not an exact registered addition".into());
    }
    manifest["add_members"] = serde_json::to_value(list).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(&folder).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut temp, &manifest).map_err(|e| e.to_string())?;
    temp.flush().map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(&manifest_path)
        .map_err(|e| e.error.to_string())?;
    Ok(())
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
        enabled: false,
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
            let base = fs::canonicalize(stage.join("paks"))
                .map_err(|_| "Configured signing stage is unavailable")?;
            let original = if regular(source, 16 * 1024 * 1024 * 1024) {
                fs::canonicalize(source).map_err(|e| e.to_string())?
            } else {
                // A sparse setup deliberately doesn't contain every original
                // PAK yet. A project can remain editable and selectable if its
                // recorded source is exactly in THIS stage's paks directory.
                // The signed file is checked and materialized at Play time.
                let status = read_json(&stage.join("stage_status.json"), 128 * 1024)?;
                if status["status"].as_str() != Some("fully-staged-offline")
                    || source
                        .components()
                        .any(|x| matches!(x, std::path::Component::ParentDir))
                    || !source.starts_with(&base)
                {
                    return Err("Original staged archive is unavailable".into());
                }
                let relative = source
                    .strip_prefix(&base)
                    .map_err(|_| "Project source is outside its stage")?
                    .to_string_lossy()
                    .replace('\\', "/");
                let plan = read_json(&stage.join("rekey_plan.json"), 32 * 1024 * 1024)?;
                if !plan["entries"].as_array().is_some_and(|rows| {
                    rows.iter().any(|entry| {
                        entry["path"].as_str() == Some(relative.as_str())
                            && entry["kind"].as_str() == Some("signed-encrypted")
                    })
                }) {
                    return Err(
                        "Project source is missing from authenticated signing inventory".into(),
                    );
                }
                source.to_path_buf()
            };
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
        // Registered additions are deliberately first: huge unchanged source
        // PAKs must not hide a newly added UI asset in the dashboard budget.
        for name in additions(&manifest, entries)? {
            let path = folder.join("files").join(&name);
            if !regular(&path, 16 * 1024 * 1024 * 1024) {
                known = false;
                continue;
            }
            let canonical = fs::canonicalize(&path).map_err(|e| e.to_string())?;
            if !canonical.starts_with(&files) {
                return Err("Added project file escaped its workspace".into());
            }
            let size = fs::metadata(&path).map_err(|e| e.to_string())?.len();
            if checked.saturating_add(size) > hash_budget {
                known = false;
                continue;
            }
            checked += size;
            changed.push(name);
        }
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
                    "{} changed asset(s) found; some files are missing or exceed the fast-scan budget. Build Mod will fully recheck them",
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

/// User-facing names and switches are separate from signed PAK manifests.
/// Existing projects default to OFF so an upgrade never silently deploys mods.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ModPreference {
    name: Option<String>,
    enabled: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ModPreferences {
    schema: u32,
    mods: HashMap<String, ModPreference>,
}

impl Default for ModPreferences {
    fn default() -> Self {
        Self {
            schema: 1,
            mods: HashMap::new(),
        }
    }
}

fn preference_key(root: &Path, workspace: &Path) -> Result<String, String> {
    let base = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let folder = fs::canonicalize(workspace).map_err(|e| e.to_string())?;
    let relative = folder
        .strip_prefix(&base)
        .map_err(|_| "Mod is outside Projects")?;
    let name = relative.to_string_lossy().replace('\\', "/");
    if !safe_member(&name) {
        return Err("Invalid mod folder".into());
    }
    Ok(name)
}

fn read_mod_preferences(root: &Path) -> Result<ModPreferences, String> {
    let path = root.join("mod-settings.json");
    if !path.exists() {
        return Ok(ModPreferences::default());
    }
    let settings: ModPreferences = serde_json::from_value(read_json(&path, 4 * 1024 * 1024)?)
        .map_err(|e| format!("Cannot read saved mods: {e}"))?;
    if settings.schema != 1 || settings.mods.len() > MAX_PROJECTS {
        return Err("Saved mod settings are unsupported".into());
    }
    Ok(settings)
}

/// Persist only the requested UI setting; never alter an extracted PAK, game,
/// signing key, or recovery journal.
pub fn save_mod_preference(
    root: &Path,
    workspace: &Path,
    name: Option<&str>,
    enabled: Option<bool>,
) -> Result<(), String> {
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let key = preference_key(&root, workspace)?;
    let mut settings = read_mod_preferences(&root)?;
    let entry = settings.mods.entry(key).or_default();
    if let Some(value) = name {
        let trimmed = value.trim();
        if trimmed.is_empty()
            || trimmed.chars().count() > 80
            || trimmed.chars().any(char::is_control)
        {
            return Err("Give the mod a name between 1 and 80 characters".into());
        }
        entry.name = Some(trimmed.into());
    }
    if let Some(value) = enabled {
        entry.enabled = value;
    }
    let mut temporary = tempfile::NamedTempFile::new_in(&root).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut temporary, &settings).map_err(|e| e.to_string())?;
    temporary.flush().map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary
        .persist(root.join("mod-settings.json"))
        .map_err(|e| e.error.to_string())?;
    Ok(())
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
    let settings = read_mod_preferences(&root)?;
    for project in &mut projects {
        let key = preference_key(&root, &project.workspace)?;
        if let Some(prefs) = settings.mods.get(&key) {
            if let Some(name) = &prefs.name {
                project.label = name.clone();
            }
            project.enabled = prefs.enabled;
        }
    }
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
    edit_imported_snapshot_with_progress(pak, projects, None)
}

pub fn edit_imported_snapshot_with_progress(
    pak: &evolve_core::ImportedPak,
    projects: &Path,
    progress: Option<&ProjectCopyProgress>,
) -> Result<(PathBuf, usize), String> {
    if let Some(progress) = progress {
        progress.phase.store(0, Ordering::Relaxed);
    }
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
    let expected = original["entries"]
        .as_array()
        .ok_or("Imported file list missing")?;
    if expected.len() != inventory.entries.len()
        || expected.is_empty()
        || expected.len() > MAX_ENTRIES
    {
        return Err("Imported archive has an invalid file index".into());
    }
    // Index manifest records once instead of a linear search per asset.
    let mut by_path = HashMap::with_capacity(expected.len());
    for record in expected {
        let path = record["path"]
            .as_str()
            .ok_or("Imported archive manifest has an invalid member name")?;
        if !safe_member(path) || by_path.insert(path, record).is_some() {
            return Err("Imported archive manifest has duplicate or unsafe members".into());
        }
    }
    if let Some(progress) = progress {
        progress
            .files_total
            .store(inventory.entries.len(), Ordering::Relaxed);
        progress
            .bytes_total
            .store(inventory.total_bytes, Ordering::Relaxed);
        progress.phase.store(1, Ordering::Relaxed);
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
        let checksum = copy_verified_member(&source, &copied, progress)?;
        let original_record = by_path
            .get(rel)
            .copied()
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
        if let Some(progress) = progress {
            progress.files_done.fetch_add(1, Ordering::Relaxed);
        }
    }
    if let Some(progress) = progress {
        progress.phase.store(2, Ordering::Relaxed);
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
        .write(true)
        .create_new(true)
        .open(staging.path().join(".evolve-pak-workspace.json"))
        .map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut file, &manifest).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    let stem: String = pak
        .name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(70)
        .collect();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
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

/// Sign edited files from an imported snapshot against the *currently*
/// installed source PAK. Source version differences are not editing errors:
/// the current archive's authenticated member table decides whether this
/// edit can be represented. Never write into the installation or stage here.
fn build_imported_snapshot(
    folder: &Path,
    stage: &Path,
    parent: &Path,
    manifest: &Value,
    modified: &[String],
    verified: ProjectRow,
) -> Result<(String, PathBuf, ProjectRow), String> {
    let source_name = manifest["imported_source"]
        .as_str()
        .ok_or("Imported project has no original installation path")?;
    let input = Path::new(source_name);
    if !regular(input, 16 * 1024 * 1024 * 1024) {
        return Err("The source PAK is unavailable. Editing is still supported; choose an installed source PAK before producing a signed game archive.".into());
    }
    let source = fs::canonicalize(input).map_err(|e| e.to_string())?;
    let physical_name = super::modding_keys::signed_basename(&source)?;
    if physical_name != verified.target {
        return Err(
            "The game's PAK name differs from the imported project's original archive".into(),
        );
    }
    let official = super::official_stage2_tools::for_stage(stage)?;
    let signer = if official.is_some() {
        None
    } else {
        Some(super::modding_keys::signing_keys(stage).map_err(|_| {
            "Editing is available without setup. Run Set Up Manager to configure a signed Stage 2 build.".to_owned()
        })?)
    };
    let originals = super::modding_keys::archive_verification_keys(stage)?;
    let lowercase = physical_name.to_ascii_lowercase();
    let signed_names = [physical_name.as_str(), lowercase.as_str()];
    let mut authenticated = None;
    for name in signed_names {
        for key in &originals.paths {
            if evolve_core::inspect_signed_pak(&source, key, name).is_ok() {
                authenticated = Some((key.clone(), name.to_owned()));
                break;
            }
        }
        if authenticated.is_some() {
            break;
        }
    }
    let (original_key, signed_name) = authenticated.ok_or(
        "This source PAK cannot be authenticated for signing. The editor is still available; select the correct game installation or rebuild its signing setup."
    )?;
    let index = evolve_core::open_verified_signed_pak(&source, &original_key, &signed_name)
        .map_err(|e| e.to_string())?;
    let source_files = fs::canonicalize(folder.join("files")).map_err(|e| e.to_string())?;
    let serialized = tempfile::Builder::new()
        .prefix(".serialize-import-")
        .tempdir_in(folder)
        .map_err(|e| e.to_string())?;
    let entries = manifest["entries"]
        .as_array()
        .ok_or("Missing extracted member index")?;
    let added: HashSet<String> = additions(manifest, entries)?.into_iter().collect();
    if modified.len() > 64 {
        return Err("Signed build is limited to 64 changes including additions".into());
    }
    let mut replacements = Vec::with_capacity(modified.len());
    for rel in modified {
        if !safe_member(rel) {
            return Err("Unsafe project member".into());
        }
        let file = folder.join("files").join(rel);
        let canonical = fs::canonicalize(&file).map_err(|e| e.to_string())?;
        if !canonical.starts_with(&source_files) || !regular(&file, 16 * 1024 * 1024 * 1024) {
            return Err("Edited file is not inside its managed workspace".into());
        }
        let record = entries
            .iter()
            .find(|row| row["path"].as_str() == Some(rel.as_str()));
        if added.contains(rel) {
            if index
                .info
                .entries
                .iter()
                .any(|row| row.path.eq_ignore_ascii_case(rel))
            {
                return Err(format!(
                    "Current source PAK already contains added member {rel}; re-import instead of replacing it"
                ));
            }
            replacements.push((rel.clone(), file));
        } else if editable_format(
            record.ok_or("Selected file is missing from the imported snapshot")?,
        )? == "cryxml"
        {
            let prior = index
                .read_member(&source, rel, MAX_XML_BYTES as u64)
                .map_err(|e| e.to_string())?;
            let encoded = evolve_core::cryxml::encode_edited_xml(
                &prior,
                &fs::read(&file).map_err(|e| e.to_string())?,
                MAX_XML_BYTES,
            )
            .map_err(|e| e.to_string())?;
            let path = serialized
                .path()
                .join(format!("member-{}.bin", replacements.len()));
            fs::write(&path, encoded).map_err(|e| e.to_string())?;
            replacements.push((rel.clone(), path));
        } else {
            replacements.push((rel.clone(), file));
        }
    }
    let built = parent.join("Built");
    if !built.exists() {
        fs::create_dir(&built).map_err(|e| e.to_string())?;
    }
    if !fs::symlink_metadata(&built).is_ok_and(|m| m.file_type().is_dir()) {
        return Err("Built directory is linked or unsafe".into());
    }
    let number = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let output_folder = built.join(format!("Import-Edit-{number:x}"));
    fs::create_dir(&output_folder).map_err(|e| e.to_string())?;
    let signed_folder = output_folder.join("signed");
    fs::create_dir(&signed_folder).map_err(|e| e.to_string())?;
    let signed_path = signed_folder.join(&signed_name);
    let output = output_folder.join(&physical_name);
    let rebuilt: Result<usize, String> = if !added.is_empty() {
        // Explicit additions use the independently verified signed-member
        // builder. The official Stage 2 encoder supports replacements only.
        super::modding_keys::signing_keys(stage)
            .map_err(|_| "Registered additions require locally configured signing keys; the official encoder currently supports replacements only".to_owned())
            .and_then(|signer| {
                evolve_core::signed_pak::rebuild_signed_pak_with_additions(
                    &source, &original_key, &signer.private, &signer.public,
                    &signed_name, &replacements, &signed_path,
                ).map(|report| report.len()).map_err(|e| e.to_string())
            })
    } else if let Some(tools) = official {
        // Imported source is a stock-key PAK. The official encoder requires
        // the isolated once-converted version of that exact installed PAK.
        let relative = offline_test_target(&verified, stage)?;
        let converted = stage.join("paks").join(relative);
        tools
            .build_changed_pak(&converted, &signed_name, &replacements, &signed_path)
            .map(|()| replacements.len())
    } else {
        let signer = signer.ok_or("Signing stage has no private key")?;
        evolve_core::rebuild_signed_pak_entries(
            &source,
            &original_key,
            &signer.private,
            &signer.public,
            &signed_name,
            &replacements,
            &signed_path,
        )
        .map(|report| report.len())
        .map_err(|e| e.to_string())
    };
    match rebuilt {
        Ok(report) => {
            fs::rename(&signed_path, &output).map_err(|e| e.to_string())?;
            fs::remove_dir(&signed_folder).map_err(|e| e.to_string())?;
            Ok((
                format!(
                    "{} modified member(s), {} registered addition(s); source version differences accepted",
                    report,
                    added.len()
                ),
                output,
                verified,
            ))
        }
        Err(error) => {
            let _ = fs::remove_dir(&signed_folder);
            let _ = fs::remove_dir(&output_folder);
            Err(format!(
                "Cannot create a signed PAK for this current game archive: {error}. Your editable project is preserved."
            ))
        }
    }
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
    if manifest["extraction_mode"].as_str() == Some("imported-snapshot") {
        return build_imported_snapshot(
            &folder,
            stage,
            &parent,
            &manifest,
            modified,
            verified.clone(),
        );
    }
    let records = manifest
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("Original manifest has no file records")?;
    let added: HashSet<String> = additions(&manifest, records)?.into_iter().collect();
    for rel in modified {
        if !safe_member(rel) {
            return Err("Unsafe modified member".into());
        }
        if !added.contains(rel) {
            let expected = records
                .iter()
                .find(|entry| entry.get("path").and_then(Value::as_str) == Some(rel.as_str()))
                .ok_or("Modified member is absent from the verified original manifest")?;
            editable_format(expected)?;
        }
    }
    // Standalone Build and Play both provision only this edited PAK on
    // demand. An unedited project never forces a costly staged copy.
    provision_selected_original(project, stage)?;
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
    let official = super::official_stage2_tools::for_stage(stage)?;
    let public = super::modding_keys::public_key(stage)?;
    if let Some(hash) = manifest.get("public_key_sha256").and_then(Value::as_str)
        && !digest(&public)
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
    let original_index = evolve_core::open_verified_signed_pak(&original, &public, signed_name)
        .map_err(|e| e.to_string())?;
    for rel in &added {
        if original_index
            .info
            .entries
            .iter()
            .any(|entry| entry.path.eq_ignore_ascii_case(rel))
        {
            return Err(format!(
                "Source PAK already contains added member {rel}; re-extract this project instead"
            ));
        }
    }
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
            .find(|entry| entry.get("path").and_then(Value::as_str) == Some(rel.as_str()));
        if added.contains(rel) {
            replacements.push((rel.clone(), replacement));
        } else if editable_format(record.ok_or("Modified member is absent from manifest")?)?
            == "cryxml"
        {
            if !regular(&replacement, MAX_XML_BYTES as u64) {
                return Err("Edited XML exceeds document limit".into());
            }
            let prior = original_index
                .read_member(&original, rel, MAX_XML_BYTES as u64)
                .map_err(|e| e.to_string())?;
            let encoded = serialize_project_xml(
                &prior,
                &fs::read(&replacement).map_err(|e| e.to_string())?,
                record.ok_or("Modified member is absent from manifest")?,
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
    let output_folder = if official.is_some() && added.is_empty() {
        // Reuse only a signature-verified output built from identical original
        // PAK, member contents, verified public key, tool version and basename.
        let mut identity = Sha256::new();
        identity.update(b"official-stage2-2025-encode-v1");
        identity.update(baseline.as_bytes());
        identity.update(digest(&public).map_err(|e| e.to_string())?.as_bytes());
        identity.update(signed_name.as_bytes());
        let mut members = replacements
            .iter()
            .map(|(name, file)| {
                digest(file)
                    .map(|hash| (name.clone(), hash))
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        members.sort_by(|left, right| left.0.cmp(&right.0));
        for (name, hash) in members {
            identity.update(name.as_bytes());
            identity.update([0]);
            identity.update(hash.as_bytes());
            identity.update([0]);
        }
        built
            .join("OfficialCache")
            .join(format!("{:x}", identity.finalize()))
    } else {
        let number = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        built.join(format!("Rust-Raw-{:x}", number))
    };
    fs::create_dir_all(&output_folder).map_err(|e| e.to_string())?;
    let output = output_folder.join(&basename);
    if official.is_some() && added.is_empty() && output.exists() {
        evolve_core::inspect_signed_pak(&output, &public, signed_name)
            .map_err(|e| format!("Official PAK cache is not authenticated: {e}"))?;
        return Ok((
            format!("{} modified member(s) · cached", replacements.len()),
            output,
            verified,
        ));
    }
    // The core signs using the authenticated original basename. If retail
    // CryPAK uses ASCII lowercase while the installed file uses mixed case,
    // first publish the signed file under that verified spelling, then move
    // it to its original physical basename within the same new output folder.
    let signed_output = output_folder.join(signed_name);
    let result: Result<usize, String> = if !added.is_empty() {
        super::modding_keys::signing_keys(stage)
            .map_err(|_| "Registered additions require locally configured signing keys; the official encoder currently supports replacements only".to_owned())
            .and_then(|keys| {
                evolve_core::signed_pak::rebuild_signed_pak_with_additions(
                    &original, &keys.public, &keys.private, &keys.public,
                    signed_name, &replacements, &signed_output,
                ).map(|report| report.len()).map_err(|e| e.to_string())
            })
    } else if let Some(tools) = official {
        tools
            .build_changed_pak(&original, signed_name, &replacements, &signed_output)
            .map(|()| replacements.len())
    } else {
        let keys = super::modding_keys::signing_keys(stage)?;
        evolve_core::rebuild_signed_pak_entries(
            &original,
            &keys.public,
            &keys.private,
            &keys.public,
            signed_name,
            &replacements,
            &signed_output,
        )
        .map(|report| report.len())
        .map_err(|e| e.to_string())
    };
    match result {
        Ok(reports) => {
            if signed_output != output {
                fs::rename(&signed_output, &output).map_err(|e| {
                    format!("Cannot restore the original PAK filename after signing: {e}")
                })?;
            }
            Ok((
                format!(
                    "{} modified member(s), {} registered addition(s)",
                    reports,
                    added.len()
                ),
                output,
                verified,
            ))
        }
        Err(error) => {
            // The core uses an atomic tempfile on failure. Remove only our
            // new empty output directory, never an existing user's path.
            let _ = fs::remove_dir(output_folder);
            Err(error)
        }
    }
}

/// Provision only the installed PAK targeted by this enabled Mod Project.
/// The verified setup inventory supplies the exact path and source digest;
/// no basename-only lookup across the game is allowed.
pub fn provision_selected_original(project: &ProjectRow, stage: &Path) -> Result<(), String> {
    let manifest = read_json(
        &project.workspace.join(".evolve-pak-workspace.json"),
        8 * 1024 * 1024,
    )?;
    let paks =
        fs::canonicalize(stage.join("paks")).map_err(|_| "Signing stage has no paks directory")?;
    let relative = if manifest["extraction_mode"].as_str() == Some("imported-snapshot") {
        let plan = read_json(&stage.join("rekey_plan.json"), 8 * 1024 * 1024)?;
        let root = fs::canonicalize(
            plan["source_root"]
                .as_str()
                .ok_or("Signing inventory has no game root")?,
        )
        .map_err(|e| e.to_string())?;
        let source = fs::canonicalize(
            manifest["imported_source"]
                .as_str()
                .ok_or("Imported project is missing its original PAK location")?,
        )
        .map_err(|e| e.to_string())?;
        source
            .strip_prefix(&root)
            .map_err(|_| "Imported project belongs to a different game installation")?
            .to_string_lossy()
            .replace('\\', "/")
    } else {
        let source = Path::new(
            manifest["source_pak"]
                .as_str()
                .ok_or("Editable project has no staged original path")?,
        );
        if source
            .components()
            .any(|p| matches!(p, std::path::Component::ParentDir))
        {
            return Err("Editable source contains unsafe parent traversal".into());
        }
        source
            .strip_prefix(&paks)
            .map_err(|_| "This project belongs to a different signing stage")?
            .to_string_lossy()
            .replace('\\', "/")
    };
    super::modding_keys::ensure_selected_stage_pak(stage, &relative)
}

/// Resolve an exact installed-game target for an offline test. This does
/// NOT require an old source checksum to match a game update, but never
/// guesses a destination by basename or searches outside the selected game.
pub(super) fn offline_test_target(project: &ProjectRow, stage: &Path) -> Result<String, String> {
    let manifest = read_json(
        &project.workspace.join(".evolve-pak-workspace.json"),
        8 * 1024 * 1024,
    )?;
    if manifest["extraction_mode"].as_str() != Some("imported-snapshot") {
        return super::modding_keys::staged_target(
            stage,
            &stage.join("paks").join(&project.target),
        );
    }
    let source = manifest["imported_source"]
        .as_str()
        .ok_or("Imported snapshot is missing its installed source location")?;
    let plan = read_json(&stage.join("rekey_plan.json"), 8 * 1024 * 1024)?;
    let root = plan["source_root"]
        .as_str()
        .ok_or("Stage has no installed game root")?;
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let source = fs::canonicalize(source).map_err(|_| {
        "Imported source is unavailable. Choose a current game PAK before installing.".to_owned()
    })?;
    let name = source.strip_prefix(&root)
        .map_err(|_| "This snapshot belongs to another game installation; select its matching offline stage")?
        .to_string_lossy().replace('\\', "/");
    if !safe_member(&name) || !name.to_ascii_lowercase().ends_with(".pak") {
        return Err("Unsafe installed target path".into());
    }
    if !plan["entries"].as_array().is_some_and(|entries| {
        entries.iter().any(|entry| {
            entry["path"].as_str() == Some(name.as_str())
                && entry["kind"].as_str() == Some("signed-encrypted")
        })
    }) {
        return Err(
            "The selected installed PAK is not present in this stage's verified target list".into(),
        );
    }
    let target = stage.join("paks").join(&name);
    if super::modding_keys::staged_target(stage, &target)? != name {
        return Err("Stage target differs from the installed game target".into());
    }
    if target.file_name() != source.file_name() {
        return Err("Installed source and stage target basenames differ".into());
    }
    Ok(name)
}

/// Offline preparation only verifies and stages a PAK already signed by the
/// configured backend. Do not demand a Rust private key when the official
/// Stage 2 encoder has already produced the authenticated archive.
fn offline_test_verification_keys(stage: &Path) -> Result<(PathBuf, PathBuf), String> {
    Ok((
        super::modding_keys::original_public_key(stage)?,
        super::modding_keys::public_key(stage)?,
    ))
}

/// Prepare a newly built signed archive for offline testing through the
/// existing profile transaction, with all stage originals backed up BEFORE
/// any staged PAK is replaced. The live game installation is untouched.
pub fn prepare_built_offline_test(
    project: &ProjectRow,
    output: &Path,
    stage: &Path,
) -> Result<String, String> {
    let target = offline_test_target(project, stage)?;
    let basename = super::modding_keys::signed_basename(output)?;
    if target.split('/').next_back() != Some(basename.as_str()) {
        return Err("Built PAK basename differs from installed stage destination".into());
    }
    // This is a verification/staging operation: no private key is needed.
    // Official Stage 2 signing stages intentionally contain the encoder and
    // public key, but not a manager-generated private_key.pem.
    let (original_public, public) = offline_test_verification_keys(stage)?;
    // Verify the exact built archive and stage target before persisting
    // anything. Signed member inventory differences are permitted offline.
    let lowercase = basename.to_ascii_lowercase();
    evolve_core::inspect_signed_pak(output, &public, &basename)
        .or_else(|_| evolve_core::inspect_signed_pak(output, &public, &lowercase))
        .map_err(|e| format!("Built offline PAK signature could not be verified: {e}"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let display = basename
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(24)
        .collect::<String>();
    let name = format!("Offline Test {display} {:x}", stamp);
    let profile = evolve_core::create_mod_profile(stage, &name)
        .map_err(|e| format!("Cannot create offline test profile: {e}"))?;
    let staged = (|| -> Result<(), String> {
        evolve_core::add_mod_to_profile(
            stage,
            &profile.id,
            output,
            &target,
            &original_public,
            &public,
            &basename,
        )
        .map_err(|e| e.to_string())?;
        evolve_core::apply_profile_stage(stage, &profile.id, &original_public, &public)
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(error) = staged {
        let _ = evolve_core::profiles::delete_mod_profile(stage, &profile.id);
        return Err(format!(
            "Offline test was not prepared: {error}. Restore any pending stage transaction before retrying."
        ));
    }
    Ok(format!(
        "Offline test prepared for {target}. Select Play with Mods to test the changed menu. Stage originals are saved in the recovery journal; use Restore staged profile when finished. Compatibility with this game build is not guaranteed."
    ))
}

/// A member must be a regular, manifest-listed extracted text source.
/// This intentionally rejects Lua bytecode, binary CryXML, and arbitrary
/// filesystem paths; contents are checked separately before presenting them.
pub fn valid_text_member(member: &str) -> bool {
    let lower = member.to_ascii_lowercase();
    safe_member(member)
        && [".xml", ".lua", ".txt"]
            .iter()
            .any(|extension| lower.ends_with(extension))
}

/// Save typed UTF-8 source through the SAME journaled, verified backup/import
/// path as an external replacement. A tempfile outside the project supplies
/// the replacement; expected_current prevents overwriting concurrent edits.
/// Never edits the signed source archive or Asset Library extraction.
pub fn save_text_member(
    project: &ProjectRow,
    projects_root: &Path,
    member: &str,
    expected_current: &[u8],
    edited: &str,
) -> Result<bool, String> {
    if !valid_text_member(member) || edited.len() > 2 * 1024 * 1024 {
        return Err("Text editor accepts only safe XML/Lua/TXT up to 2 MiB".into());
    }
    if edited.contains('\0') {
        return Err("Binary NUL bytes cannot be saved as editable source text".into());
    }
    let mut replacement = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    replacement
        .write_all(edited.as_bytes())
        .map_err(|e| e.to_string())?;
    replacement
        .as_file_mut()
        .sync_all()
        .map_err(|e| e.to_string())?;
    import_raw_member_checked(
        project,
        projects_root,
        member,
        replacement.path(),
        Some(expected_current),
    )
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
    if !safe_member(member) {
        return Err("Replacement target must be an existing safe project member".into());
    }
    let lower = member.to_ascii_lowercase();
    let png = replacement_source
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    if png && (lower.ends_with(".dds") || lower.ends_with(".dds.0")) {
        // Conversion never touches the managed source directly. A separate
        // temporary DDS enters the same authenticated backup/import transaction.
        // Broken, incompatible or streaming texture sets fail before editing.
        let original = project.workspace.join("files").join(member);
        let temporary = tempfile::Builder::new()
            .suffix(".dds")
            .tempfile()
            .map_err(|e| e.to_string())?;
        png_dds_import::convert(replacement_source, &original, temporary.path())?;
        import_raw_member_checked(project, projects_root, member, temporary.path(), None)
    } else if png && lower.contains(".dds.") {
        Err("Split DDS streaming fragments require a complete native texture set; single PNG conversion is unsupported.".into())
    } else {
        import_raw_member_checked(project, projects_root, member, replacement_source, None)
    }
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
    fn explicit_additions_are_detected_without_turning_existing_files_into_additions() {
        let temp = tempfile::tempdir().unwrap();
        let stage = temp.path().join("staged");
        let pak = stage.join("paks/Game/UI_Data.pak");
        fs::create_dir_all(pak.parent().unwrap()).unwrap();
        fs::write(&pak, b"unchanged source").unwrap();
        let projects = temp.path().join("Projects");
        let folder = projects.join("Project");
        let files = folder.join("files");
        fs::create_dir_all(files.join("Libs/UI")).unwrap();
        fs::write(files.join("Libs/UI/stock.gfx"), b"stock").unwrap();
        fs::write(files.join("Libs/UI/extra.gfx"), b"new").unwrap();
        fs::write(folder.join(".evolve-pak-workspace.json"), serde_json::to_vec(&serde_json::json!({
            "version":1,"source_pak":pak,
            "entries":[{"path":"Libs/UI/stock.gfx","format":"raw","sha256":format!("{:x}", Sha256::digest(b"stock"))}]
        })).unwrap()).unwrap();
        let before = inspect_project(&folder, &stage, "Project".into(), "".into(), u64::MAX);
        assert_eq!(before.status, "Draft");
        register_added_member(&projects, &before, "Libs/UI/extra.gfx").unwrap();
        assert!(register_added_member(&projects, &before, "libs/ui/EXTRA.gfx").is_err());
        assert!(register_added_member(&projects, &before, "Libs/UI/stock.gfx").is_err());
        assert!(register_added_member(&projects, &before, "../outside.gfx").is_err());
        let after = inspect_project(&folder, &stage, "Project".into(), "".into(), u64::MAX);
        assert_eq!(after.modified, vec!["Libs/UI/extra.gfx"]);
        assert!(after.changed_known);
        assert!(unregister_added_member(&projects, &before, "libs/UI/extra.gfx").is_err());
        unregister_added_member(&projects, &before, "Libs/UI/extra.gfx").unwrap();
        assert_eq!(
            inspect_project(&folder, &stage, "Project".into(), "".into(), u64::MAX).status,
            "Draft"
        );
        assert!(files.join("Libs/UI/extra.gfx").is_file());
    }
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
            enabled: false,
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
    fn effectset_perks_are_indexed_edited_and_backed_up() {
        let temp = tempfile::tempdir().unwrap();
        let projects = temp.path().join("projects");
        let workspace = projects.join("Perks");
        let member = "Libs/Items/Perks/MPerkBrawlerHighEffect.effectset";
        let file = workspace.join("files").join(member);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let original = br#"<effectSet name="MPerkBrawlerHighEffect" levels="3"><effect class="GameProperty"><param name="value" value="0.06|0.07|0.09"/><param name="bUITreatAsPercentage" value="True"/></effect></effectSet>"#;
        fs::write(&file, original).unwrap();
        fs::write(workspace.join(".evolve-pak-workspace.json"), serde_json::to_vec(&serde_json::json!({
            "version":1, "entries":[{"path":member,"format":"raw", "sha256":format!("{:x}",Sha256::digest(original))}]
        })).unwrap()).unwrap();
        let project = ProjectRow {
            label: "Perks".into(),
            target: "items.pak".into(),
            source_build: "Test".into(),
            status: "Draft".into(),
            details: String::new(),
            workspace: workspace.clone(),
            modified: vec![],
            changed_known: true,
            enabled: false,
        };
        assert_eq!(guided_members(&project, false).unwrap(), vec![member]);
        assert!(guided_members(&project, true).unwrap().is_empty());
        let snapshot = read_gameplay_document(&project, member).unwrap();
        let mut fields = evolve_core::gameplay_values::fields(&snapshot).unwrap();
        assert_eq!(fields.len(), 3);
        fields[1].value = "0.07|0.08|0.10".into();
        fields[2].value = "False".into();
        assert!(save_gameplay_document(&project, &projects, member, &snapshot, &fields).unwrap());
        let saved = fs::read_to_string(&file).unwrap();
        assert!(saved.contains(r#"value="0.07|0.08|0.10""#));
        assert!(saved.contains(r#"value="False""#));
        assert!(save_gameplay_document(&project, &projects, member, &snapshot, &fields).is_err());
        assert_eq!(
            fs::read_dir(workspace.join("EditorBackups"))
                .unwrap()
                .count(),
            1
        );
        assert!(restore_original_raw_member(&project, &projects, member).unwrap());
        assert_eq!(fs::read(&file).unwrap(), original);
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
            enabled: false,
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
            enabled: false,
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
    fn mod_name_and_enable_switch_persist_without_touching_original_assets() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("projects");
        let folder = root.join("Main Menu");
        fs::create_dir_all(folder.join("files")).unwrap();
        let original = folder.join("files/main_menu.gfx");
        fs::write(&original, b"untouched original snapshot").unwrap();
        assert!(save_mod_preference(&root, &folder, Some(""), None).is_err());
        save_mod_preference(&root, &folder, Some("  My Main Menu  "), Some(true)).unwrap();
        let prefs = read_mod_preferences(&root).unwrap();
        let item = prefs.mods.get("Main Menu").unwrap();
        assert_eq!(item.name.as_deref(), Some("My Main Menu"));
        assert!(item.enabled);
        save_mod_preference(&root, &folder, None, Some(false)).unwrap();
        let prefs = read_mod_preferences(&root).unwrap();
        assert!(!prefs.mods.get("Main Menu").unwrap().enabled);
        assert_eq!(fs::read(&original).unwrap(), b"untouched original snapshot");
        assert!(save_mod_preference(&root, temp.path(), None, Some(true)).is_err());
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
    #[test]
    fn official_manual_offline_test_requires_only_public_verification_keys() {
        let temp = tempfile::tempdir().unwrap();
        let stage = temp.path().join("staged");
        let keys = stage.join("mykeys");
        fs::create_dir_all(&keys).unwrap();
        fs::write(temp.path().join("RSAKeyData.bin"), [1u8; 140]).unwrap();
        fs::write(keys.join("public_key.bin"), [2u8; 140]).unwrap();

        // Real official-tool stages have no manager-generated private PEM.
        assert!(!keys.join("private_key.pem").exists());
        assert!(super::super::modding_keys::signing_keys(&stage).is_err());
        let (original, custom) = offline_test_verification_keys(&stage).unwrap();
        assert_eq!(original, temp.path().join("RSAKeyData.bin"));
        assert_eq!(custom, keys.join("public_key.bin"));
        fs::remove_file(&custom).unwrap();
        assert!(offline_test_verification_keys(&stage).is_err());
    }
}
