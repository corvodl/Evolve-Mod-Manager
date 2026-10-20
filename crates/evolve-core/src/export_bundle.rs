//! Exact indexed model export, staged atomically in a user-named folder.
//! Source DDS files remain untouched; external texture exports are PNG only.
use crate::{
    AnimationPolicy, DependencyStatus, EquipmentOverrides, Inventory,
    convert_glb_to_fbx_with_options, export_dds_png, export_model_glb,
    export_rigged_character_fbx_options, export_unified_cdf_fbx_options,
    inspect_dependencies, read_model, read_static_cdf_assembly,
};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Component, Path, PathBuf};

const MAX_EXPORTS: usize = 64;
const MAX_REFERENCES: usize = 2048;
const MAX_COPY_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ExportBundleOptions {
    pub destination: PathBuf,
    pub selected: Vec<String>,
    pub blender: PathBuf,
    pub skeleton: Option<PathBuf>,
    pub diffuse: Option<PathBuf>,
    pub caf: Option<PathBuf>,
    pub rigged: bool,
    pub embed_textures: bool,
    pub export_dependencies: bool,
    pub equipment: EquipmentOverrides,
}

#[derive(Clone, Debug)]
pub struct ExportBundleReport {
    pub destination: PathBuf,
    pub exported: Vec<String>,
    pub png_textures: usize,
    pub other_dependencies: usize,
    pub warnings: Vec<String>,
}

fn relative_path(path: &str) -> Result<&Path, String> {
    let result = Path::new(path);
    if path.is_empty() || path.contains('\\') || result.is_absolute()
        || !result.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err("Expected an exact indexed, non-traversing asset path".into());
    }
    Ok(result)
}

fn indexed_file(root: &Path, inventory: &Inventory, relative: &str) -> Result<PathBuf, String> {
    let part = relative_path(relative)?;
    let indexed = inventory.entries.iter()
        .find(|entry| Path::new(&entry.relative_path) == part)
        .ok_or_else(|| format!("Asset was not in this index: {relative}"))?;
    let source = root.join(part);
    let meta = fs::symlink_metadata(&source).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() != indexed.byte_size
        || meta.len() > MAX_COPY_BYTES
    {
        return Err(format!("Asset changed or is unsafe: {relative}"));
    }
    let checked = fs::canonicalize(source).map_err(|e| e.to_string())?;
    if !checked.starts_with(root) {
        return Err("Asset escaped indexed source directory".into());
    }
    Ok(checked)
}

fn output_stem(relative: &str) -> Result<String, String> {
    let stem = relative_path(relative)?.file_stem()
        .ok_or("Source has no file stem")?.to_string_lossy();
    let result: String = stem.chars().map(|c| {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }
    }).collect();
    if result.is_empty() || result.len() > 90 || result.chars().all(|c| c == '_') {
        return Err("Invalid output filename".into());
    }
    Ok(result)
}

fn png_relative_path(relative: &str) -> Result<PathBuf, String> {
    let path = relative_path(relative)?;
    let lower = relative.to_ascii_lowercase();
    let suffix = if lower.ends_with(".dds.0") { 6 }
        else if lower.ends_with(".dds") { 4 }
        else { return Err(format!("Unsupported texture conversion: {relative}")); };
    let base = &relative[..relative.len() - suffix];
    let mut result = PathBuf::from(format!("{base}.png"));
    // All path components are still validated as indexed relative paths.
    relative_path(&result.to_string_lossy())?;
    if result.extension().is_none_or(|e| !e.eq_ignore_ascii_case("png")) {
        return Err("External texture must be a PNG".into());
    }
    // Avoid a warning about mutable path; conversion never changes source.
    result.shrink_to_fit();
    let _ = path;
    Ok(result)
}

fn validate_target(root: &Path, target: &Path) -> Result<PathBuf, String> {
    if !target.is_absolute() || target.exists() {
        return Err("Choose a new, absolute, user-named export folder".into());
    }
    let name = target.file_name().ok_or("Export folder has no name")?
        .to_string_lossy();
    if name.is_empty() || name == "." || name == ".."
        || name.ends_with(' ') || name.ends_with('.')
        || name.chars().any(|c| c.is_control() || r#"<>:"/\|?*"#.contains(c))
    {
        return Err("Export folder name is not valid on Windows".into());
    }
    let parent = target.parent().ok_or("Export folder has no parent")?;
    let meta = fs::symlink_metadata(parent).map_err(|e| e.to_string())?;
    if !meta.file_type().is_dir() {
        return Err("Parent must be a real existing directory".into());
    }
    let parent = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    if parent.starts_with(root) {
        return Err("Exporting into the original asset folder is not permitted".into());
    }
    Ok(parent)
}

fn dependencies(
    root: &Path, inventory: &Inventory, selection: &[String], bundle: &Path,
) -> Result<(usize, usize), String> {
    let mut queue: VecDeque<String> = selection.iter().cloned().collect();
    let mut visited = HashSet::new();
    let mut files = HashSet::new();
    let mut output_names = HashSet::new();
    let mut copied = 0u64;
    let mut pngs = 0usize;
    let mut others = 0usize;
    while let Some(relative) = queue.pop_front() {
        if !visited.insert(relative.to_ascii_lowercase()) { continue; }
        if visited.len() > MAX_REFERENCES {
            return Err("Dependency graph exceeds safe limit".into());
        }
        let lower = relative.to_ascii_lowercase();
        if !(lower.ends_with(".cdf") || lower.ends_with(".mtl")) { continue; }
        let graph = inspect_dependencies(inventory, &relative)
            .map_err(|e| format!("Cannot resolve {relative} dependencies: {e}"))?;
        for node in graph.nodes.iter().skip(1) {
            if node.status != DependencyStatus::Resolved || node.candidates.len() != 1 {
                return Err(format!("Missing/ambiguous dependency {} in {relative}", node.label));
            }
            let dep = &node.candidates[0];
            let key = dep.to_ascii_lowercase();
            if !files.insert(key) { continue; }
            if files.len() > MAX_REFERENCES {
                return Err("Too many dependency files".into());
            }
            let source = indexed_file(root, inventory, dep)?;
            let size = fs::metadata(&source).map_err(|e| e.to_string())?.len();
            copied = copied.checked_add(size).ok_or("Dependency copy overflow")?;
            if copied > MAX_COPY_BYTES { return Err("Dependencies exceed 1 GiB budget".into()); }
            let texture = node.kind.eq_ignore_ascii_case("Texture")
                || key.ends_with(".dds") || key.ends_with(".dds.0");
            let destination = if texture {
                bundle.join("textures").join(png_relative_path(dep)?)
            } else {
                bundle.join("dependencies").join(relative_path(dep)?)
            };
            let out_key = destination.to_string_lossy().to_ascii_lowercase();
            if !output_names.insert(out_key) {
                return Err(format!("Two source dependencies share an output name: {dep}"));
            }
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            if texture {
                export_dds_png(&source, &destination).map_err(|e| {
                    format!("Could not convert {dep} to PNG; no original DDS was copied: {e}")
                })?;
                pngs += 1;
            } else {
                fs::copy(&source, &destination).map_err(|e| e.to_string())?;
                others += 1;
            }
            if key.ends_with(".mtl") {
                queue.push_back(dep.clone());
            }
        }
    }
    Ok((pngs, others))
}

/// Publish no half-finished exports: Blender and PNG conversions happen in a
/// temporary sibling directory, renamed into place only after verification.
pub fn export_bundle(
    inventory: &Inventory, options: &ExportBundleOptions,
) -> Result<ExportBundleReport, String> {
    let root = fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
    let parent = validate_target(&root, &options.destination)?;
    if options.selected.is_empty() || options.selected.len() > MAX_EXPORTS {
        return Err(format!("Choose 1 to {MAX_EXPORTS} models or CDF assemblies"));
    }
    let mut seen = HashSet::new();
    let mut filenames = HashSet::new();
    let mut selected = Vec::new();
    for relative in &options.selected {
        let source = indexed_file(&root, inventory, relative)?;
        if !seen.insert(relative.to_ascii_lowercase()) {
            return Err("The same asset was selected twice".into());
        }
        let lower = relative.to_ascii_lowercase();
        if !(lower.ends_with(".cdf") || [".skin", ".skinm", ".chr", ".chrm",
            ".cgf", ".cgfm", ".cga", ".cgam"].iter().any(|s| lower.ends_with(s)))
        {
            return Err(format!("Not a supported model or assembly: {relative}"));
        }
        let filename = format!("{}.fbx", output_stem(relative)?);
        if !filenames.insert(filename.to_ascii_lowercase()) {
            return Err(format!("Several assets would overwrite {filename}"));
        }
        selected.push((relative.clone(), source, filename));
    }
    let stage = tempfile::Builder::new().prefix(".evolve-model-export-")
        .tempdir_in(&parent).map_err(|e| e.to_string())?;
    let bundle = stage.path().join("bundle");
    fs::create_dir(&bundle).map_err(|e| e.to_string())?;
    let mut exported = Vec::new();
    let mut warnings = Vec::new();
    for (relative, source, filename) in selected {
        let target = bundle.join(&filename);
        if options.rigged && relative.to_ascii_lowercase().ends_with(".cdf") {
            export_unified_cdf_fbx_options(
                &source, inventory, &options.blender, &target,
                options.caf.as_deref(), &options.equipment, options.embed_textures,
            ).map_err(|e| format!("{relative}: {e}"))?;
        } else if options.rigged {
            let skeleton = options.skeleton.as_ref()
                .ok_or("Rigged standalone exports need a selected original skeleton")?;
            let meta = fs::symlink_metadata(skeleton).map_err(|e| e.to_string())?;
            if !meta.file_type().is_file()
                || !fs::canonicalize(skeleton).map_err(|e| e.to_string())?.starts_with(&root)
            {
                return Err("Skeleton must be an original regular file within the indexed root".into());
            }
            export_rigged_character_fbx_options(
                &source, skeleton, &options.blender,
                options.diffuse.as_deref(), options.caf.as_deref(), &target,
                options.embed_textures,
            ).map_err(|e| format!("{relative}: {e}"))?;
        } else {
            let mesh = if relative.to_ascii_lowercase().ends_with(".cdf") {
                read_static_cdf_assembly(&source, inventory).map_err(|e| e.to_string())?.0
            } else { read_model(&source).map_err(|e| e.to_string())? };
            let glb = stage.path().join(format!("{}.glb", output_stem(&relative)?));
            export_model_glb(&mesh, &glb).map_err(|e| e.to_string())?;
            convert_glb_to_fbx_with_options(
                &options.blender, &glb, &target, AnimationPolicy::default(),
                options.embed_textures,
            ).map_err(|e| e.to_string())?;
            if options.embed_textures {
                warnings.push(format!("{relative}: static geometry has no verified embedded DDS material maps"));
            }
        }
        exported.push(filename);
    }
    let (png_textures, other_dependencies) = if options.export_dependencies {
        let counts = dependencies(&root, inventory, &options.selected, &bundle)?;
        fs::create_dir_all(bundle.join("textures")).map_err(|e| e.to_string())?;
        fs::create_dir_all(bundle.join("dependencies")).map_err(|e| e.to_string())?;
        counts
    } else { (0, 0) };
    if options.destination.exists() {
        return Err("Export folder was created by another process".into());
    }
    fs::rename(&bundle, &options.destination).map_err(|e| e.to_string())?;
    Ok(ExportBundleReport {
        destination: options.destination.clone(), exported, png_textures,
        other_dependencies, warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_names_replace_dds_suffix_and_preserve_structure() {
        assert_eq!(png_relative_path("textures/Scorpid.dds.0").unwrap(),
            PathBuf::from("textures/Scorpid.png"));
        assert_eq!(png_relative_path("textures/Scorpid.dds").unwrap(),
            PathBuf::from("textures/Scorpid.png"));
        assert!(png_relative_path("textures/Scorpid.tif").is_err());
    }
    #[test]
    fn disallow_traversal_and_existing_folder() {
        assert!(relative_path("../escape.cdf").is_err());
        assert!(relative_path("characters/body.cdf").is_ok());
        let original = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let created = destination.path().join("existing");
        fs::create_dir(&created).unwrap();
        assert!(validate_target(original.path(), &created).is_err());
        assert!(validate_target(original.path(), &destination.path().join("chosen")).is_ok());
    }
}
