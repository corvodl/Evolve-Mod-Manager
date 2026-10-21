//! Isolated GFX editing sessions, following the private manager's FFDec flow.
//! No operation modifies original PAKs, libraries or the game installation.
use crate::modding_projects::{self, ProjectRow};
use evolve_core::gfx::{GfxInfo, MAX_SOURCE, inspect_gfx};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_XML: u64 = 64 * 1024 * 1024;
const MAX_REPLACEMENT: u64 = 32 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(75);

#[derive(Debug, Clone)]
pub struct Session {
    pub source: PathBuf,
    pub relative: String,
    pub editable: PathBuf,
    pub folder: Arc<tempfile::TempDir>,
    pub project: Option<ProjectRow>,
    pub original_sha: String,
    pub info: GfxInfo,
    pub edits: usize,
    pub applied: bool,
}
#[derive(Debug, Clone)]
pub enum Action {
    OpenGui,
    ReloadGui,
    ImportGfx(PathBuf),
    ReplaceResource { index: usize, replacement: PathBuf },
    ExportXml,
    ImportXml,
    ExportScripts,
    ImportScripts,
    Undo,
    Apply { projects_root: PathBuf },
    PreviewFrame,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn regular(path: &Path, maximum: u64) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() > maximum {
        return Err(format!("Not a bounded regular file: {}", path.display()));
    }
    Ok(())
}
fn read_gfx(path: &Path) -> Result<(Vec<u8>, GfxInfo), String> {
    regular(path, MAX_SOURCE as u64)?;
    let data = fs::read(path).map_err(|e| e.to_string())?;
    let info = inspect_gfx(&data).map_err(|e| e.to_string())?;
    Ok((data, info))
}
fn valid_member(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 2048
        && !name.contains([':', '\\'])
        && !name.starts_with('/')
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}
fn project_allows(project: &ProjectRow, source: &Path, member: &str) -> Result<(), String> {
    if !valid_member(member) || !member.to_ascii_lowercase().ends_with(".gfx") {
        return Err("Only named GFX project members may be edited".into());
    }
    let folder = fs::canonicalize(&project.workspace).map_err(|e| e.to_string())?;
    let files = fs::canonicalize(folder.join("files")).map_err(|e| e.to_string())?;
    let source = fs::canonicalize(source).map_err(|e| e.to_string())?;
    if source != files.join(member) || !source.starts_with(&files) {
        return Err("GFX source is not in this editable project's files".into());
    }
    regular(&folder.join(".evolve-pak-workspace.json"), 8 * 1024 * 1024)?;
    let manifest: Value = serde_json::from_slice(
        &fs::read(folder.join(".evolve-pak-workspace.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if manifest.get("version").and_then(Value::as_u64) != Some(1)
        || manifest.get("reference_only").and_then(Value::as_bool) == Some(true)
    {
        return Err("Project is not an editable Stage 2 PAK workspace".into());
    }
    let entries = manifest
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("Project has no validated entries")?;
    if entries
        .iter()
        .filter(|e| e.get("path").and_then(Value::as_str) == Some(member))
        .count()
        != 1
    {
        return Err("Selected GFX is not uniquely listed in the project manifest".into());
    }
    Ok(())
}
pub fn create(
    source: PathBuf,
    relative: String,
    project: Option<ProjectRow>,
) -> Result<Session, String> {
    if !source.is_absolute()
        || !valid_member(&relative)
        || !relative.to_ascii_lowercase().ends_with(".gfx")
    {
        return Err("Select a verified extracted .gfx file".into());
    }
    if let Some(project) = &project {
        project_allows(project, &source, &relative)?;
    }
    let (bytes, info) = read_gfx(&source)?;
    let folder = Arc::new(
        tempfile::Builder::new()
            .prefix("evolve-private-gfx-edit-")
            .tempdir()
            .map_err(|e| e.to_string())?,
    );
    let editable = folder.path().join("movie.gfx");
    fs::write(&editable, &bytes).map_err(|e| e.to_string())?;
    Ok(Session {
        source,
        relative,
        editable,
        folder,
        project,
        original_sha: digest(&bytes),
        info,
        edits: 0,
        applied: false,
    })
}
fn ensure_unchanged(session: &Session) -> Result<(), String> {
    let (data, _) = read_gfx(&session.source)?;
    if digest(&data) != session.original_sha {
        return Err("Extracted GFX changed since this editing session began; reopen it".into());
    }
    Ok(())
}
fn stage(session: &mut Session, bytes: &[u8]) -> Result<(), String> {
    let info = inspect_gfx(bytes).map_err(|e| e.to_string())?;
    if digest(bytes) == digest(&fs::read(&session.editable).map_err(|e| e.to_string())?) {
        return Err("FFDec returned an unchanged movie".into());
    }
    if session.edits >= 100 {
        return Err("GFX edit history limit reached; apply or reopen the project".into());
    }
    regular(&session.editable, MAX_SOURCE as u64)?;
    let previous = session
        .folder
        .path()
        .join(format!("undo-{:03}.gfx", session.edits));
    fs::copy(&session.editable, &previous).map_err(|e| e.to_string())?;
    let pending = session.folder.path().join("rebuilt.pending");
    fs::write(&pending, bytes).map_err(|e| e.to_string())?;
    fs::rename(&pending, &session.editable).map_err(|e| e.to_string())?;
    session.edits += 1;
    session.info = info;
    session.applied = false;
    Ok(())
}
fn ffdec_command(config: &str) -> Result<Command, String> {
    let path = Path::new(config.trim());
    if !path.is_absolute() {
        return Err("Configure an absolute FFDec 26.3.0 ffdec-cli.exe or ffdec.jar path".into());
    }
    regular(path, 150 * 1024 * 1024)?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    if ext.eq_ignore_ascii_case("jar") {
        let mut cmd = Command::new("java");
        cmd.arg("-jar").arg(path);
        Ok(cmd)
    } else if ext.eq_ignore_ascii_case("exe")
        && path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("ffdec-cli.exe"))
    {
        Ok(Command::new(path))
    } else {
        Err("FFDec path must point to ffdec-cli.exe or ffdec.jar".into())
    }
}
fn run_cli(config: &str, folder: &Path, arguments: &[String]) -> Result<(), String> {
    let mut cmd = ffdec_command(config)?;
    let mut child = cmd
        .arg("-cli")
        .arg("-onerror")
        .arg("abort")
        .args(arguments)
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Unable to run FFDec: {e}"))?;
    let since = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!(
                    "FFDec failed (exit status {status}); original GFX remains untouched"
                ))
            };
        }
        if since.elapsed() >= TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err("FFDec timed out after 75 seconds".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn bounded_xml(input: &Path) -> Result<(), String> {
    regular(input, MAX_XML)?;
    let mut file = fs::File::open(input).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_XML + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let content = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
    if ["<!doctype", "<!entity", "<?xml-stylesheet"]
        .iter()
        .any(|token| content.contains(token))
    {
        return Err("FFDec XML must not contain external entities, DTD or stylesheet".into());
    }
    Ok(())
}
fn import_compiled(session: &mut Session, candidate: &Path) -> Result<(), String> {
    let (bytes, _) = read_gfx(candidate)?;
    stage(session, &bytes)
}
/// Private manager parity: work only on expendable copies until explicit Apply.
pub fn run(
    mut session: Session,
    action: Action,
    configured: &str,
) -> Result<(Session, String, Option<Vec<u8>>), String> {
    let root = session.folder.path().to_path_buf();
    let mut preview = None;
    let message = match action {
        Action::OpenGui => {
            ffdec_command(configured)?;
            let working = root.join("external-ffdec.gfx");
            fs::copy(&session.editable, &working).map_err(|e| e.to_string())?;
            let configured_path = Path::new(configured.trim());
            let launcher = if configured_path
                .extension()
                .is_some_and(|e| e.to_str().is_some_and(|e| e.eq_ignore_ascii_case("jar")))
            {
                let mut command = Command::new("java");
                command.arg("-jar").arg(configured_path);
                command
            } else {
                let parent = configured_path
                    .parent()
                    .ok_or("Invalid FFDec installation path")?;
                let exe = parent.join("ffdec.exe");
                if !exe.is_file() {
                    return Err("Full FFDec GUI needs ffdec.exe beside ffdec-cli.exe".into());
                }
                Command::new(exe)
            };
            let mut launcher = launcher;
            launcher
                .arg(&working)
                .current_dir(&root)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| e.to_string())?;
            "FFDec GUI launched on a private copy. Save, then click Reload FFDec changes."
                .to_owned()
        }
        Action::ReloadGui => {
            import_compiled(&mut session, &root.join("external-ffdec.gfx"))?;
            "Validated external FFDec edit; original project remains unchanged.".to_owned()
        }
        Action::ImportGfx(candidate) => {
            import_compiled(&mut session, &candidate)?;
            "Imported checked external GFX into private session; project unchanged.".to_owned()
        }
        Action::ReplaceResource { index, replacement } => {
            let tag = session
                .info
                .resources
                .get(index)
                .ok_or("Select an existing movie resource")?
                .clone();
            let ext = replacement
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let supported = match tag.kind {
                "image" => ["png", "jpg", "jpeg"].contains(&ext.as_str()),
                "shape" => ext == "svg",
                "text" => ext == "txt",
                "sound" => ["mp3", "wav"].contains(&ext.as_str()),
                "binary" => ext == "bin",
                _ => false,
            };
            if !supported {
                return Err("Resource type and replacement extension do not match".into());
            }
            regular(&replacement, MAX_REPLACEMENT)?;
            let before = root.join("replace-before.gfx");
            let output = root.join("replace-result.gfx");
            fs::copy(&session.editable, &before).map_err(|e| e.to_string())?;
            let args = vec![
                "-replace".into(),
                before.display().to_string(),
                output.display().to_string(),
                tag.id.to_string(),
                replacement.display().to_string(),
            ];
            run_cli(configured, &root, &args)?;
            import_compiled(&mut session, &output)?;
            format!(
                "Rebuilt {} resource #{} through FFDec; project unchanged.",
                tag.kind, tag.id
            )
        }
        Action::ExportXml => {
            let target = root.join("edited-tags.xml");
            let args = vec![
                "-swf2xml".into(),
                session.editable.display().to_string(),
                target.display().to_string(),
            ];
            run_cli(configured, &root, &args)?;
            bounded_xml(&target)?;
            format!(
                "Exported editable tag XML: {}. Modify it before Rebuild.",
                target.display()
            )
        }
        Action::ImportXml => {
            let input = root.join("edited-tags.xml");
            bounded_xml(&input)?;
            let output = root.join("xml-rebuilt.gfx");
            let args = vec![
                "-xml2swf".into(),
                input.display().to_string(),
                output.display().to_string(),
            ];
            run_cli(configured, &root, &args)?;
            import_compiled(&mut session, &output)?;
            "Compiled edited tag XML; staged only in private GFX session.".to_owned()
        }
        Action::ExportScripts => {
            let target = root.join("ActionScript");
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            let args = vec![
                "-export".into(),
                "script".into(),
                target.display().to_string(),
                session.editable.display().to_string(),
            ];
            run_cli(configured, &root, &args)?;
            format!("Exported ActionScript for editing: {}", target.display())
        }
        Action::ImportScripts => {
            let input = root.join("ActionScript");
            if !input.is_dir() {
                return Err("Export editable ActionScript first".into());
            }
            let before = root.join("scripts-before.gfx");
            fs::copy(&session.editable, &before).map_err(|e| e.to_string())?;
            let output = root.join("scripts-rebuilt.gfx");
            let args = vec![
                "-importScript".into(),
                before.display().to_string(),
                output.display().to_string(),
                input.display().to_string(),
            ];
            run_cli(configured, &root, &args)?;
            import_compiled(&mut session, &output)?;
            "Compiled edited ActionScript; staged only in private GFX session.".to_owned()
        }
        Action::Undo => {
            if session.edits == 0 {
                return Err("No staged GFX edits to undo".into());
            }
            let previous = root.join(format!("undo-{:03}.gfx", session.edits - 1));
            let (bytes, info) = read_gfx(&previous)?;
            fs::copy(&previous, &session.editable).map_err(|e| e.to_string())?;
            fs::remove_file(previous).map_err(|e| e.to_string())?;
            session.edits -= 1;
            session.info = info;
            session.applied = false;
            if bytes.is_empty() {
                return Err("Undo snapshot was empty".into());
            }
            "Reverted the last staged GFX edit; extracted project unchanged.".to_owned()
        }
        Action::Apply { projects_root } => {
            let project = session.project.as_ref().ok_or(
                "Asset Library is read-only. Open this movie from an editable Mod Project.",
            )?;
            ensure_unchanged(&session)?;
            let (now, _) = read_gfx(&session.editable)?;
            if digest(&now) == session.original_sha {
                return Err("No edits to apply".into());
            }
            let changed = modding_projects::import_raw_member(
                project,
                &projects_root,
                &session.relative,
                &session.editable,
            )?;
            if !changed {
                return Err("Project was already byte-identical".into());
            }
            session.original_sha = digest(&now);
            session.applied = true;
            "Applied validated GFX to extracted Mod Project with EditorBackups; no installed PAK changed.".to_owned()
        }
        Action::PreviewFrame => {
            let output = root.join("preview");
            fs::create_dir_all(&output).map_err(|e| e.to_string())?;
            let args = vec![
                "-select".into(),
                "1".into(),
                "-format".into(),
                "frame:png".into(),
                "-importAssets".into(),
                "yes,local".into(),
                "-export".into(),
                "frame".into(),
                output.display().to_string(),
                session.editable.display().to_string(),
            ];
            run_cli(configured, &root, &args)?;
            let pictures = fs::read_dir(&output)
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
                .map(|item| item.path())
                .filter(|p| {
                    p.extension()
                        .is_some_and(|x| x.to_str().is_some_and(|x| x.eq_ignore_ascii_case("png")))
                })
                .collect::<Vec<_>>();
            if pictures.len() != 1 {
                return Err("FFDec did not export exactly one selected frame PNG".into());
            }
            regular(&pictures[0], 70 * 1024 * 1024)?;
            preview = Some(fs::read(&pictures[0]).map_err(|e| e.to_string())?);
            "FFDec rendered the first static frame. Runtime ActionScript is not executed."
                .to_owned()
        }
    };
    Ok((session, message, preview))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn safe_member_rejects_traversal_and_windows_drive_paths() {
        assert!(valid_member("Libs/UI/main_menu.gfx"));
        for path in [
            "../main.gfx",
            "/main.gfx",
            "C:/main.gfx",
            "a\\b.gfx",
            "a//b.gfx",
        ] {
            assert!(!valid_member(path));
        }
    }
    #[test]
    fn external_ffdec_must_be_absolute_and_supported() {
        assert!(ffdec_command("ffdec-cli.exe").is_err());
        assert!(ffdec_command("C:/not-installed/ffdec-cli.exe").is_err());
    }
    #[test]
    fn xml_dtd_and_external_entities_are_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("edited-tags.xml");
        fs::write(
            &file,
            b"<!DOCTYPE x [<!ENTITY y SYSTEM 'file:///etc/passwd'>]>",
        )
        .unwrap();
        assert!(bounded_xml(&file).is_err());
        fs::write(&file, b"<swf><tags /></swf>").unwrap();
        assert!(bounded_xml(&file).is_ok());
    }
}
