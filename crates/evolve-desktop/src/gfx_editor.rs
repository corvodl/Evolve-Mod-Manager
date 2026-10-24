//! Isolated GFX editing sessions, following the private manager's FFDec flow.
//! No operation modifies original PAKs, libraries or the game installation.
use crate::gfx_dependencies::{self, Dependency};
use crate::gfx_external_textures;
use crate::gfx_featured_promo::{self, Mode as FeaturedMode, Target as FeaturedTarget};
use crate::gfx_legacy_layout;
use crate::gfx_menu_map;
use crate::modding_projects::{self, ProjectRow};
use evolve_core::gfx::{GfxInfo, MAX_SOURCE, inspect_gfx};
use evolve_core::gfx_layout::{LayoutEdit, StaticLayer, edit_first_frame, static_layers};
use evolve_core::{LibraryReferenceStatus, imported_library, resolve_imported_reference};
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
    pub layers: Vec<StaticLayer>,
    pub dependencies: Vec<Dependency>,
    pub menu_commands: Vec<gfx_menu_map::MenuCommand>,
    pub menu_command_status: String,
    pub library_root: Option<PathBuf>,
    pub source_build: Option<String>,
    pub edits: usize,
    pub applied: bool,
}
#[derive(Debug, Clone)]
pub enum Action {
    OpenGui,
    ReloadGui,
    ImportGfx(PathBuf),
    ReplaceResource {
        index: usize,
        replacement: PathBuf,
    },
    ReplaceText {
        index: usize,
        text: String,
    },
    ExportXml,
    ImportXml,
    ExportScripts,
    ImportScripts,
    Undo,
    Apply {
        projects_root: PathBuf,
    },
    PreviewFrame,
    StageLayout(LayoutEdit),
    StageLegacyRows,
    Featured {
        mode: FeaturedMode,
        target: FeaturedTarget,
        text: String,
    },
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// When the editor was opened outside the Asset Library's build-filtered view,
/// use only the exact original GFX bytes of a verified library member to
/// recover its build label. One ambiguous or cross-build match never becomes
/// an inferred build; no filename-only Stage 2 fallback is permitted.
fn verified_gfx_source_build(root: &Path, relative: &str, source_bytes: &[u8]) -> Option<String> {
    let catalog = imported_library(root).ok()?;
    if catalog.builds.len() > 32 {
        return None;
    }
    let mut result: Option<String> = None;
    for label in catalog.builds {
        let Ok(LibraryReferenceStatus::Found(found)) =
            resolve_imported_reference(root, relative, Some(&label))
        else {
            continue;
        };
        let path = Path::new(&found.absolute_path);
        let Ok(meta) = fs::symlink_metadata(path) else {
            continue;
        };
        if !meta.file_type().is_file()
            || meta.len() != source_bytes.len() as u64
            || meta.len() > MAX_SOURCE as u64
        {
            continue;
        }
        let Ok(candidate) = fs::read(path) else {
            continue;
        };
        if candidate == source_bytes {
            if result.is_some() {
                // Identical source movie assigned to multiple build labels:
                // user must select the build explicitly.
                return None;
            }
            result = Some(label);
        }
    }
    result
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
    imported_library: Option<PathBuf>,
    source_build: Option<String>,
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
    let source_build = source_build.or_else(|| {
        imported_library
            .as_deref()
            .and_then(|root| verified_gfx_source_build(root, &relative, &bytes))
    });
    let folder = Arc::new(
        tempfile::Builder::new()
            .prefix("evolve-private-gfx-edit-")
            .tempdir()
            .map_err(|e| e.to_string())?,
    );
    let editable = folder.path().join("movie.gfx");
    fs::write(&editable, &bytes).map_err(|e| e.to_string())?;
    let dependencies = gfx_dependencies::stage_imports(
        &info.imports,
        &relative,
        &source,
        folder.path(),
        imported_library.as_deref(),
        source_build.as_deref(),
    )?;
    let (menu_commands, menu_command_status) = if relative
        .eq_ignore_ascii_case("Libs/UI/main_menu.gfx")
    {
        gfx_menu_map::load(imported_library.as_deref(), source_build.as_deref())
            .unwrap_or_else(|error| (Vec::new(), format!("UI_Data mapping unavailable: {error}")))
    } else {
        (Vec::new(), String::new())
    };
    Ok(Session {
        source,
        relative,
        editable,
        folder,
        project,
        original_sha: digest(&bytes),
        layers: static_layers(&bytes).map_err(|e| e.to_string())?,
        dependencies,
        menu_commands,
        menu_command_status,
        library_root: imported_library,
        source_build,
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
    let layers = static_layers(bytes).map_err(|e| e.to_string())?;
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
    session.layers = layers;
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
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // FFDec's explicit Open GUI command remains visible. Only CLI work,
        // including "Render frame", is console-less.
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
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
/// FFDec ActionScript imports must remain inside the isolated session.
/// Reject symlinks, unrelated files, unbounded trees, and empty sources.
fn validate_script_tree(root: &Path) -> Result<(), String> {
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let mut stack = vec![root.clone()];
    let (mut count, mut total) = (0usize, 0u64);
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            let path = entry.path();
            if kind.is_symlink() || !path.starts_with(&root) {
                return Err("ActionScript export contains a linked/unsafe path".into());
            }
            if kind.is_dir() {
                stack.push(path);
                if stack.len() > 256 {
                    return Err("ActionScript directory nesting exceeds limit".into());
                }
            } else if kind.is_file() {
                if !path
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x.eq_ignore_ascii_case("as"))
                {
                    return Err("Only ActionScript .as source files may be rebuilt".into());
                }
                count += 1;
                total += entry.metadata().map_err(|e| e.to_string())?.len();
                if count > 2000 || total > MAX_XML {
                    return Err("ActionScript source tree exceeds 2000 files/64 MiB".into());
                }
            } else {
                return Err("Unsupported ActionScript source type".into());
            }
        }
    }
    if count == 0 {
        return Err("No ActionScript source files found".into());
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
        Action::ReplaceText { index, text } => {
            let tag = session
                .info
                .resources
                .get(index)
                .ok_or("Select an original text resource")?;
            if tag.kind != "text" {
                return Err(
                    "Direct text editing requires a selected existing text resource".into(),
                );
            }
            let character_id = tag.id;
            if session.project.is_none() {
                return Err(
                    "Asset Library movies are read-only; open an editable Mod Project".into(),
                );
            }
            if text.is_empty()
                || text.len() > 4096
                || text.contains('\0')
                || text
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                return Err("Enter 1–4096 printable UTF-8 bytes (newlines allowed)".into());
            }
            let before = root.join("text-before.gfx");
            let replacement = root.join("replacement-text.txt");
            let output = root.join("text-rebuilt.gfx");
            fs::write(&replacement, &text).map_err(|e| e.to_string())?;
            fs::copy(&session.editable, &before).map_err(|e| e.to_string())?;
            run_cli(
                configured,
                &root,
                &[
                    "-replace".into(),
                    before.display().to_string(),
                    output.display().to_string(),
                    character_id.to_string(),
                    replacement.display().to_string(),
                ],
            )?;
            import_compiled(&mut session, &output)?;
            format!(
                "Staged text resource #{} replacement; Undo or Apply to Mod Project.",
                character_id
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
            validate_script_tree(&input)?;
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
        Action::Featured { mode, target, text } => {
            if !session
                .relative
                .eq_ignore_ascii_case("Libs/UI/main_menu.gfx")
            {
                return Err(
                    "Featured promotion editing is only available for Libs/UI/main_menu.gfx".into(),
                );
            }
            if session.project.is_none() {
                return Err("Open main_menu.gfx from an editable Mod Project first; Asset Library files are read-only".into());
            }
            // FFDec operates exclusively on a new temporary session copy, not
            // on the source movie or game installation. Each script is checked
            // before it can be compiled and staged with the normal Undo path.
            let work = tempfile::Builder::new()
                .prefix("featured-promo-")
                .tempdir_in(&root)
                .map_err(|e| e.to_string())?;
            let scripts = work.path().join("scripts");
            fs::create_dir(&scripts).map_err(|e| e.to_string())?;
            run_cli(
                configured,
                work.path(),
                &[
                    "-export".into(),
                    "script".into(),
                    scripts.display().to_string(),
                    session.editable.display().to_string(),
                ],
            )?;
            validate_script_tree(&scripts)?;
            let mut pending = vec![scripts.clone()];
            let mut matches = Vec::new();
            let canonical = fs::canonicalize(&scripts).map_err(|e| e.to_string())?;
            while let Some(folder) = pending.pop() {
                for entry in fs::read_dir(&folder).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let path = entry.path();
                    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                    if !fs::canonicalize(&path)
                        .map_err(|e| e.to_string())?
                        .starts_with(&canonical)
                    {
                        return Err("ActionScript source escaped the private workspace".into());
                    }
                    if metadata.file_type().is_dir() {
                        pending.push(path);
                    } else if metadata.file_type().is_file()
                        && path
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("as"))
                    {
                        let input = fs::read_to_string(&path).map_err(|e| e.to_string())?;
                        if gfx_featured_promo::has_definition(&input) {
                            matches.push((path, input));
                        }
                    } else {
                        return Err("Unexpected ActionScript export member".into());
                    }
                }
            }
            if matches.len() != 1 {
                return Err(
                    "Expected exactly one exported setFeatured script; no changes staged".into(),
                );
            }
            let (script_path, original) = matches.pop().unwrap();
            let edited = gfx_featured_promo::patch(&original, mode, target, &text)?;
            fs::write(&script_path, edited).map_err(|e| e.to_string())?;
            let before = work.path().join("before.gfx");
            fs::copy(&session.editable, &before).map_err(|e| e.to_string())?;
            let output = work.path().join("rebuilt.gfx");
            run_cli(
                configured,
                work.path(),
                &[
                    "-importScript".into(),
                    before.display().to_string(),
                    output.display().to_string(),
                    scripts.display().to_string(),
                ],
            )?;
            regular(&output, MAX_SOURCE as u64)?;
            let (rebuilt, _) = read_gfx(&output)?;
            stage(&mut session, &rebuilt)?;
            format!(
                "Staged persistent featured-promotion {} for {}. Rotating offers are rechecked on each frame; Undo or Apply to Mod Project. Offline game verification remains necessary.",
                match mode {
                    FeaturedMode::Hide => "hide",
                    FeaturedMode::Remove => "remove/disable",
                    FeaturedMode::Text => "text replacement",
                },
                target.label()
            )
        }
        Action::StageLegacyRows => {
            if !session.relative.eq_ignore_ascii_case("Libs/UI/main_menu.gfx") {
                return Err("Legacy row preset only supports Libs/UI/main_menu.gfx".into());
            }
            if session.project.is_none() {
                return Err("Open Stage 2 main_menu.gfx from an editable Mod Project".into());
            }
            let (before, _) = read_gfx(&session.editable)?;
            let updated = gfx_legacy_layout::legacy_main_menu_rows(&before)?;
            stage(&mut session, &updated)?;
            "Staged Legacy 70px menu row preset on Stage 2 original GFX: 40/65px default/featured rows to 70px. Original navigation, callbacks, other tags, and textures are unchanged. Undo or Apply to Mod Project; game verification required.".to_owned()
        }
        Action::StageLayout(edit) => {
            let (before, _) = read_gfx(&session.editable)?;
            let rewritten = edit_first_frame(&before, edit).map_err(|e| e.to_string())?;
            stage(&mut session, &rewritten)?;
            format!(
                "Staged depth {} / character {} transform in isolated GFX copy. Use Undo or Apply to Mod Project.",
                edit.depth, edit.character
            )
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
            session.layers = static_layers(&bytes).map_err(|e| e.to_string())?;
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
            // FFDec cannot natively resolve CryEngine/Scaleform tag 1008/1009
            // DDS atlas characters. Restore those characters only in a
            // temporary render copy, using source-build-qualified UI_Assets.
            let original = fs::read(&session.editable).map_err(|e| e.to_string())?;
            let textures = gfx_external_textures::prepare_preview(
                &original,
                &session.relative,
                &session.source,
                session.library_root.as_deref(),
                session.source_build.as_deref(),
            )?;
            let render_file = root.join("textured-preview.gfx");
            fs::write(&render_file, &textures.movie).map_err(|e| e.to_string())?;
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
                render_file.display().to_string(),
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
            format!(
                "FFDec static frame: {} verified UI_Assets bitmap tiles; {} unresolved/skipped. {} Runtime ActionScript is not executed.",
                textures.converted,
                textures.skipped,
                textures
                    .ui_data
                    .as_deref()
                    .unwrap_or("No associated same-build UI_Data element."),
            )
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
    fn auto_build_requires_exact_verified_bytes_and_unique_membership() {
        let library = tempfile::tempdir().unwrap();
        let id = "11111111111111111111";
        let folder = library.path().join(format!("ui_data.pak__{id}"));
        fs::create_dir_all(folder.join("files/Libs/UI")).unwrap();
        let movie = b"GFX  original verified movie bytes";
        fs::write(folder.join("files/Libs/UI/main_menu.gfx"), movie).unwrap();
        fs::write(
            folder.join("asset-library.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1, "name": "ui_data.pak", "source": "/unused",
                "entries": [{
                    "path": "Libs/UI/main_menu.gfx",
                    "bytes": movie.len()
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let manifest = |second: bool| {
            serde_json::json!({
                "version": 1,
                "builds": {
                    "stage-two": {
                        "label": "Stage 2",
                        "members": {"11111111111111111111": {}}
                    },
                    "legacy": {
                        "label": "Legacy",
                        "members": if second {serde_json::json!({"11111111111111111111": {}})}
                                   else {serde_json::json!({})}
                    }
                }
            })
        };
        fs::write(
            library.path().join("build-collections.json"),
            serde_json::to_vec(&manifest(false)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            verified_gfx_source_build(library.path(), "Libs/UI/main_menu.gfx", movie),
            Some("Stage 2".into())
        );
        assert_eq!(
            verified_gfx_source_build(library.path(), "Libs/UI/main_menu.gfx", b"wrong"),
            None
        );
        fs::write(
            library.path().join("build-collections.json"),
            serde_json::to_vec(&manifest(true)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            verified_gfx_source_build(library.path(), "Libs/UI/main_menu.gfx", movie),
            None
        );
    }
    #[test]
    fn external_ffdec_must_be_absolute_and_supported() {
        assert!(ffdec_command("ffdec-cli.exe").is_err());
        assert!(ffdec_command("C:/not-installed/ffdec-cli.exe").is_err());
    }
    #[test]
    fn rejects_actionscript_tree_with_other_files_or_empty_contents() {
        let dir = tempfile::tempdir().unwrap();
        assert!(validate_script_tree(dir.path()).is_err());
        fs::write(dir.path().join("file.bin"), b"incorrect").unwrap();
        assert!(validate_script_tree(dir.path()).is_err());
        fs::remove_file(dir.path().join("file.bin")).unwrap();
        fs::write(dir.path().join("feature.as"), b"trace('test');").unwrap();
        assert!(validate_script_tree(dir.path()).is_ok());
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
