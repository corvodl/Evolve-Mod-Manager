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
const MAX_ENTRIES: usize = 25000;
const MAX_HASHED_PER_WORKSPACE: u64 = 256 * 1024 * 1024;

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
fn inspect_project(folder: &Path, stage: &Path, label: String, source_build: String) -> ProjectRow {
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
        let manifest = read_json(&folder.join(".evolve-pak-workspace.json"), 8 * 1024 * 1024)?;
        if manifest.get("version").and_then(Value::as_u64) != Some(1)
            || manifest.get("reference_only").and_then(Value::as_bool) == Some(true)
        {
            return Err("Not a supported editable project manifest".into());
        }
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
        for entry in entries {
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
            if checked.saturating_add(size) > MAX_HASHED_PER_WORKSPACE {
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
            item.status = if !known {
                "Needs review"
            } else if modified.is_empty() {
                "Draft"
            } else {
                "Modified"
            }
            .into();
            item.details = if known {
                format!("{} changed asset(s)", modified.len())
            } else {
                "Some files could not be checked within scan limits".into()
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
                    || fs::symlink_metadata(path).is_err_or(|m| !m.file_type().is_dir())
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
            inspect_project(&path, stage, label, build)
        })
        .collect();
    projects.sort_by(|a, b| {
        a.label
            .to_ascii_lowercase()
            .cmp(&b.label.to_ascii_lowercase())
    });
    Ok(projects)
}

/// Build a single changed *raw* member only. CryXmlB needs a validated
/// round-trip serializer and multi-entry projects need an atomic multi-patch
/// rebuild; neither is silently approximated by this operation.
pub fn build_single_raw(
    project: &ProjectRow,
    projects: &Path,
    stage: &Path,
) -> Result<(String, PathBuf), String> {
    if !project.changed_known || project.modified.len() != 1 || project.status != "Modified" {
        return Err("Rust preview supports rebuilding exactly one verified modified raw member; no partial multi-file builds.".into());
    }
    let parent = fs::canonicalize(projects).map_err(|e| e.to_string())?;
    if !fs::symlink_metadata(projects).is_ok_and(|m| m.file_type().is_dir()) {
        return Err("Projects path must be a real directory".into());
    }
    let folder = fs::canonicalize(&project.workspace).map_err(|e| e.to_string())?;
    if !folder.starts_with(&parent) || folder == parent {
        return Err("Selected workspace is outside Projects".into());
    }
    let manifest = read_json(&folder.join(".evolve-pak-workspace.json"), 8 * 1024 * 1024)?;
    let rel = &project.modified[0];
    if !safe_member(rel) {
        return Err("Unsafe modified member".into());
    }
    let expected = manifest.get("entries").and_then(Value::as_array)
        .and_then(|entries| entries.iter().find(|entry|
            entry.get("path").and_then(Value::as_str) == Some(rel.as_str())
        ))
        .ok_or("Modified member is absent from the verified original manifest")?;
    if expected.get("format").and_then(Value::as_str) != Some("raw") {
        return Err("This CryXmlB/converted member requires semantic round-trip serialization, not raw signed replacement.".into());
    }
    let original = stage.join("paks").join(&project.target);
    let relative = super::modding_keys::staged_target(stage, &original)?;
    if relative != project.target {
        return Err("Manifest target and staged original disagree".into());
    }
    let manifest_source = manifest.get("source_pak").and_then(Value::as_str)
        .ok_or("Original manifest source path missing")?;
    if fs::canonicalize(manifest_source).map_err(|e| e.to_string())?
        != fs::canonicalize(&original).map_err(|e| e.to_string())?
    {
        return Err("Workspace source archive differs from the selected staged PAK".into());
    }
    let baseline = manifest.get("source_sha256").and_then(Value::as_str)
        .ok_or("Source archive identity digest missing")?;
    if !digest(&original).map_err(|e| e.to_string())?.eq_ignore_ascii_case(baseline) {
        return Err("Source PAK changed since extraction; refusing to build".into());
    }
    // A fresh inspection also detects edits that occurred after project-list
    // indexing. Do not issue a partial or stale rebuild.
    let fresh = inspect_project(
        &folder, stage, project.label.clone(), project.source_build.clone()
    );
    if !fresh.changed_known || fresh.modified != project.modified {
        return Err("Project edits changed during preparation; refresh and review before building".into());
    }
    let keys = super::modding_keys::signing_keys(stage)?;
    if let Some(hash) = manifest.get("public_key_sha256").and_then(Value::as_str) {
        if !digest(&keys.public).map_err(|e| e.to_string())?.eq_ignore_ascii_case(hash) {
            return Err("Stage signing key no longer matches the original extracted archive".into());
        }
    }
    let basename = super::modding_keys::signed_basename(&original)?;
    if let Some(name) = manifest.get("signed_basename").and_then(Value::as_str)
        && name != basename
    {
        return Err("Archive signed basename mismatch".into());
    }
    let replacement = folder.join("files").join(rel);
    let source_files = fs::canonicalize(folder.join("files"))
        .map_err(|e| e.to_string())?;
    let canonical = fs::canonicalize(&replacement).map_err(|e| e.to_string())?;
    if !canonical.starts_with(&source_files)
        || !fs::symlink_metadata(&replacement).is_ok_and(|m| m.file_type().is_file())
    {
        return Err("Replacement is not a regular file inside the selected project".into());
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
        .map_err(|e| e.to_string())?.as_nanos();
    let output_folder = built.join(format!("Rust-Raw-{:x}", number));
    fs::create_dir(&output_folder).map_err(|e| e.to_string())?;
    let output = output_folder.join(&basename);
    let result = evolve_core::rebuild_signed_pak_entry(
        &original, &keys.public, &keys.private, &keys.public,
        &basename, rel, &replacement, &output,
    );
    match result {
        Ok(report) => Ok((report.changed_entry, output)),
        Err(error) => {
            // The core uses an atomic tempfile on failure. Remove only our
            // new empty output directory, never an existing user's path.
            let _ = fs::remove_dir(output_folder);
            Err(error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
