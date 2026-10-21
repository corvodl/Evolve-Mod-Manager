//! Exact indexed model export, staged atomically in a user-named folder.
//! Source DDS files remain untouched; external texture exports are PNG only.
use crate::character_unified_fbx::standalone_authored_diffuse;
use crate::{
    AnimationPolicy, DependencyStatus, EquipmentOverrides, Inventory,
    convert_glb_to_fbx_with_options, export_dds_png, export_model_glb,
    export_rigged_character_fbx_options, export_unified_cdf_fbx_options_with_glow, inspect_dependencies,
    read_model, read_static_cdf_assembly_with_equipment,
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
    /// Source-verified full-armor monster glow, enabled in Model Options.
    pub full_armor_glow: bool,
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
    if path.is_empty()
        || path.contains('\\')
        || result.is_absolute()
        || !result
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err("Expected an exact indexed, non-traversing asset path".into());
    }
    Ok(result)
}

fn indexed_file(root: &Path, inventory: &Inventory, relative: &str) -> Result<PathBuf, String> {
    let part = relative_path(relative)?;
    let indexed = inventory
        .entries
        .iter()
        .find(|entry| Path::new(&entry.relative_path) == part)
        .ok_or_else(|| format!("Asset was not in this index: {relative}"))?;
    let source = root.join(part);
    let meta = fs::symlink_metadata(&source).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() != indexed.byte_size || meta.len() > MAX_COPY_BYTES
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
    let stem = relative_path(relative)?
        .file_stem()
        .ok_or("Source has no file stem")?
        .to_string_lossy();
    let result: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if result.is_empty() || result.len() > 90 || result.chars().all(|c| c == '_') {
        return Err("Invalid output filename".into());
    }
    Ok(result)
}

fn png_relative_path(relative: &str) -> Result<PathBuf, String> {
    let path = relative_path(relative)?;
    let lower = relative.to_ascii_lowercase();
    let suffix = if lower.ends_with(".dds.0") {
        6
    } else if lower.ends_with(".dds") {
        4
    } else {
        return Err(format!("Unsupported texture conversion: {relative}"));
    };
    let base = &relative[..relative.len() - suffix];
    let result = PathBuf::from(format!("{base}.png"));
    // All path components are still validated as indexed relative paths.
    relative_path(&result.to_string_lossy())?;
    if result
        .extension()
        .is_none_or(|e| !e.eq_ignore_ascii_case("png"))
    {
        return Err("External texture must be a PNG".into());
    }
    let _ = path;
    Ok(result)
}

fn validate_target(root: &Path, target: &Path) -> Result<PathBuf, String> {
    if !target.is_absolute() || target.exists() {
        return Err("Choose a new, absolute, user-named export folder".into());
    }
    let name = target
        .file_name()
        .ok_or("Export folder has no name")?
        .to_string_lossy();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with(' ')
        || name.ends_with('.')
        || name
            .chars()
            .any(|c| c.is_control() || r#"<>:"/\|?*"#.contains(c))
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
    root: &Path,
    inventory: &Inventory,
    selection: &[String],
    bundle: &Path,
) -> Result<(usize, usize), String> {
    let mut queue: VecDeque<String> = selection.iter().cloned().collect();
    let mut visited = HashSet::new();
    let mut files = HashSet::new();
    let mut output_names = HashSet::new();
    let mut copied = 0u64;
    let mut pngs = 0usize;
    let mut others = 0usize;
    while let Some(relative) = queue.pop_front() {
        if !visited.insert(relative.to_ascii_lowercase()) {
            continue;
        }
        if visited.len() > MAX_REFERENCES {
            return Err("Dependency graph exceeds safe limit".into());
        }
        let lower = relative.to_ascii_lowercase();
        if !(lower.ends_with(".cdf") || lower.ends_with(".mtl")) {
            // Original metadata may have a separately cooked render mesh.
            // Export the *exact* indexed sidecar rather than guessing a model
            // with a similar name elsewhere in the Asset Library.
            let compiled = if lower.ends_with(".skin") {
                Some("skinm")
            } else if lower.ends_with(".chr") {
                Some("chrm")
            } else if lower.ends_with(".cgf") {
                Some("cgfm")
            } else if lower.ends_with(".cga") {
                Some("cgam")
            } else {
                None
            };
            if let Some(extension) = compiled {
                let candidate = Path::new(&relative)
                    .with_extension(extension)
                    .to_string_lossy()
                    .replace('\\', "/");
                match crate::resolve_virtual(inventory, &candidate) {
                    crate::AssetResolution::Found(found) => {
                        queue.push_back(found.clone());
                        if files.insert(found.to_ascii_lowercase()) {
                            let mesh = indexed_file(root, inventory, &found)?;
                            let size = fs::metadata(&mesh).map_err(|e| e.to_string())?.len();
                            copied = copied.checked_add(size).ok_or("Dependency size overflow")?;
                            if copied > MAX_COPY_BYTES {
                                return Err("Dependencies exceed 1 GiB budget".into());
                            }
                            let output = bundle.join("dependencies").join(relative_path(&found)?);
                            if let Some(parent) = output.parent() {
                                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                            fs::copy(mesh, output).map_err(|e| e.to_string())?;
                            others += 1;
                        }
                    }
                    crate::AssetResolution::Missing => (),
                    crate::AssetResolution::Ambiguous(_) => {
                        return Err(format!("Ambiguous compiled mesh for {relative}"));
                    }
                }
            }
            // Standalone export also discovers exactly indexed sidecar MTL.
            if [
                ".skin", ".skinm", ".chr", ".chrm", ".cgf", ".cgfm", ".cga", ".cgam",
            ]
            .iter()
            .any(|suffix| lower.ends_with(suffix))
            {
                let candidate = Path::new(&relative)
                    .with_extension("mtl")
                    .to_string_lossy()
                    .replace('\\', "/");
                match crate::resolve_virtual(inventory, &candidate) {
                    crate::AssetResolution::Found(found) => {
                        queue.push_back(found.clone());
                        if files.insert(found.to_ascii_lowercase()) {
                            let material = indexed_file(root, inventory, &found)?;
                            let size = fs::metadata(&material).map_err(|e| e.to_string())?.len();
                            copied = copied.checked_add(size).ok_or("Dependency size overflow")?;
                            if copied > MAX_COPY_BYTES {
                                return Err("Dependencies exceed 1 GiB budget".into());
                            }
                            let output = bundle.join("dependencies").join(relative_path(&found)?);
                            if let Some(parent) = output.parent() {
                                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                            fs::copy(material, output).map_err(|e| e.to_string())?;
                            others += 1;
                        }
                    }
                    crate::AssetResolution::Missing => (),
                    crate::AssetResolution::Ambiguous(_) => {
                        return Err(format!("Ambiguous exact material for {relative}"));
                    }
                }
            }
            continue;
        }
        let graph = inspect_dependencies(inventory, &relative)
            .map_err(|e| format!("Cannot resolve {relative} dependencies: {e}"))?;
        for node in graph.nodes.iter().skip(1) {
            if node.status != DependencyStatus::Resolved || node.candidates.len() != 1 {
                return Err(format!(
                    "Missing/ambiguous dependency {} in {relative}",
                    node.label
                ));
            }
            let dep = &node.candidates[0];
            let key = dep.to_ascii_lowercase();
            if !files.insert(key.clone()) {
                continue;
            }
            if files.len() > MAX_REFERENCES {
                return Err("Too many dependency files".into());
            }
            let source = indexed_file(root, inventory, dep)?;
            let size = fs::metadata(&source).map_err(|e| e.to_string())?.len();
            copied = copied.checked_add(size).ok_or("Dependency copy overflow")?;
            if copied > MAX_COPY_BYTES {
                return Err("Dependencies exceed 1 GiB budget".into());
            }
            let texture = node.kind.eq_ignore_ascii_case("Texture")
                || key.ends_with(".dds")
                || key.ends_with(".dds.0");
            let destination = if texture {
                bundle.join("textures").join(png_relative_path(dep)?)
            } else {
                bundle.join("dependencies").join(relative_path(dep)?)
            };
            let out_key = destination.to_string_lossy().to_ascii_lowercase();
            if !output_names.insert(out_key) {
                return Err(format!(
                    "Two source dependencies share an output name: {dep}"
                ));
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
            } else if [
                ".skin", ".skinm", ".chr", ".chrm", ".cgf", ".cgfm", ".cga", ".cgam",
            ]
            .iter()
            .any(|suffix| key.ends_with(suffix))
            {
                // Queue all exact authored source mesh companions for secondary
                // material and compiled-geometry resolution.
                queue.push_back(dep.clone());
            }
        }
    }
    Ok((pngs, others))
}

/// Publish no half-finished exports: Blender and PNG conversions happen in a
/// temporary sibling directory, renamed into place only after verification.
pub fn export_bundle(
    inventory: &Inventory,
    options: &ExportBundleOptions,
) -> Result<ExportBundleReport, String> {
    let root = fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
    let parent = validate_target(&root, &options.destination)?;
    if options.selected.is_empty() || options.selected.len() > MAX_EXPORTS {
        return Err(format!(
            "Choose 1 to {MAX_EXPORTS} models or CDF assemblies"
        ));
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
        if !(lower.ends_with(".dds")
            || lower.ends_with(".dds.0")
            || lower.ends_with(".cdf")
            || [
                ".skin", ".skinm", ".chr", ".chrm", ".cgf", ".cgfm", ".cga", ".cgam",
            ]
            .iter()
            .any(|s| lower.ends_with(s)))
        {
            return Err(format!("Not a supported model or assembly: {relative}"));
        }
        let filename = if lower.ends_with(".dds") || lower.ends_with(".dds.0") {
            let as_png = png_relative_path(relative)?;
            format!("{}.png", output_stem(&as_png.to_string_lossy())?)
        } else {
            format!("{}.fbx", output_stem(relative)?)
        };
        if !filenames.insert(filename.to_ascii_lowercase()) {
            return Err(format!("Several assets would overwrite {filename}"));
        }
        selected.push((relative.clone(), source, filename));
    }
    let stage = tempfile::Builder::new()
        .prefix(".evolve-model-export-")
        .tempdir_in(&parent)
        .map_err(|e| e.to_string())?;
    let bundle = stage.path().join("bundle");
    fs::create_dir(&bundle).map_err(|e| e.to_string())?;
    let mut exported = Vec::new();
    let mut direct_pngs = 0usize;
    let mut warnings = Vec::new();
    for (relative, source, filename) in selected {
        let target = bundle.join(&filename);
        if filename.to_ascii_lowercase().ends_with(".png") {
            export_dds_png(&source, &target).map_err(|e| format!("{relative}: {e}"))?;
            direct_pngs += 1;
            exported.push(filename);
            continue;
        }
        if options.rigged && relative.to_ascii_lowercase().ends_with(".cdf") {
            export_unified_cdf_fbx_options_with_glow(
                &source,
                inventory,
                &options.blender,
                &target,
                options.caf.as_deref(),
                &options.equipment,
                options.embed_textures,
                options.full_armor_glow,
            )
            .map_err(|e| format!("{relative}: {e}"))?;
        } else if options.rigged {
            if !options.equipment.is_empty() || options.full_armor_glow {
                return Err("Model Options equipment and full-armor glow apply only to the selected CDF assembly".into());
            }
            let skeleton = options
                .skeleton
                .as_ref()
                .ok_or("Rigged standalone exports need a selected original skeleton")?;
            let meta = fs::symlink_metadata(skeleton).map_err(|e| e.to_string())?;
            if !meta.file_type().is_file()
                || !fs::canonicalize(skeleton)
                    .map_err(|e| e.to_string())?
                    .starts_with(&root)
            {
                return Err(
                    "Skeleton must be an original regular file within the indexed root".into(),
                );
            }
            // The checked box only means Blender may embed real images.
            // Resolve an original single-slot diffuse from its exact local MTL
            // when the user did not explicitly select a DDS. Never publish an
            // apparently "textured" white FBX when neither was verified.
            let diffuse = if let Some(path) = options.diffuse.as_ref() {
                Some(path.clone())
            } else if options.embed_textures {
                standalone_authored_diffuse(&root, &source, inventory)?
            } else {
                None
            };
            if options.embed_textures && diffuse.is_none() {
                return Err(format!(
                    "{relative}: Embedded textures requested, but no original diffuse DDS was found for this standalone model. Select a verified DDS using Export Settings → Select diffuse DDS, or index the original source MTL and texture first."
                ));
            }
            export_rigged_character_fbx_options(
                &source,
                skeleton,
                &options.blender,
                diffuse.as_deref(),
                options.caf.as_deref(),
                &target,
                options.embed_textures,
            )
            .map_err(|e| format!("{relative}: {e}"))?;
        } else {
            if options.full_armor_glow {
                return Err("Full-armor glow requires a rigged textured character export".into());
            }
            let mesh = if relative.to_ascii_lowercase().ends_with(".cdf") {
                // Static character export honors exactly the same verified
                // equipment changes as the live Model Options viewport.
                read_static_cdf_assembly_with_equipment(
                    &source,
                    inventory,
                    None,
                    None,
                    &options.equipment,
                )
                .map_err(|e| e.to_string())?
                .0
            } else {
                if !options.equipment.is_empty() {
                    return Err("Equipment overrides require the selected CDF assembly".into());
                }
                read_model(&source).map_err(|e| e.to_string())?
            };
            let glb = stage
                .path()
                .join(format!("{}.glb", output_stem(&relative)?));
            export_model_glb(&mesh, &glb).map_err(|e| e.to_string())?;
            convert_glb_to_fbx_with_options(
                &options.blender,
                &glb,
                &target,
                AnimationPolicy::default(),
                options.embed_textures,
            )
            .map_err(|e| e.to_string())?;
            if options.embed_textures {
                warnings.push(format!(
                    "{relative}: static geometry has no verified embedded DDS material maps"
                ));
            }
        }
        exported.push(filename);
    }
    let (png_textures, other_dependencies) = if options.export_dependencies {
        let counts = dependencies(&root, inventory, &options.selected, &bundle)?;
        fs::create_dir_all(bundle.join("textures")).map_err(|e| e.to_string())?;
        fs::create_dir_all(bundle.join("dependencies")).map_err(|e| e.to_string())?;
        counts
    } else {
        (0, 0)
    };
    if options.destination.exists() {
        return Err("Export folder was created by another process".into());
    }
    fs::rename(&bundle, &options.destination).map_err(|e| e.to_string())?;
    Ok(ExportBundleReport {
        destination: options.destination.clone(),
        exported,
        png_textures: png_textures + direct_pngs,
        other_dependencies,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_names_replace_dds_suffix_and_preserve_structure() {
        assert_eq!(
            png_relative_path("textures/Scorpid.dds.0").unwrap(),
            PathBuf::from("textures/Scorpid.png")
        );
        assert_eq!(
            png_relative_path("textures/Scorpid.dds").unwrap(),
            PathBuf::from("textures/Scorpid.png")
        );
        assert!(png_relative_path("textures/Scorpid.tif").is_err());
    }
    #[test]
    fn named_folder_texture_export_writes_png_without_blender() {
        let source = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let mut dds = vec![0u8; 128];
        dds[..4].copy_from_slice(b"DDS ");
        dds[4..8].copy_from_slice(&124u32.to_le_bytes());
        dds[8..12].copy_from_slice(&0x21007u32.to_le_bytes());
        dds[12..16].copy_from_slice(&4u32.to_le_bytes());
        dds[16..20].copy_from_slice(&4u32.to_le_bytes());
        dds[28..32].copy_from_slice(&1u32.to_le_bytes());
        dds[76..80].copy_from_slice(&32u32.to_le_bytes());
        dds[80..84].copy_from_slice(&4u32.to_le_bytes());
        dds[84..88].copy_from_slice(b"DXT1");
        dds.extend_from_slice(&[0, 248, 224, 7, 0, 0, 0, 0]);
        fs::create_dir(source.path().join("textures")).unwrap();
        fs::write(source.path().join("textures/diffuse.dds.0"), dds).unwrap();
        let inventory = crate::scan_directory(source.path()).unwrap();
        let output = parent.path().join("My Character Textures");
        let options = ExportBundleOptions {
            destination: output.clone(),
            selected: vec!["textures/diffuse.dds.0".into()],
            blender: PathBuf::new(),
            skeleton: None,
            diffuse: None,
            caf: None,
            rigged: false,
            embed_textures: false,
            export_dependencies: false,
            equipment: EquipmentOverrides::new(),
            full_armor_glow: false,
        };
        let report = export_bundle(&inventory, &options).unwrap();
        assert_eq!(report.png_textures, 1);
        assert_eq!(report.exported, vec!["diffuse.png"]);
        let image = fs::read(output.join("diffuse.png")).unwrap();
        assert!(image.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(!output.join("diffuse.dds").exists());
        assert!(source.path().join("textures/diffuse.dds.0").exists());
        assert!(export_bundle(&inventory, &options).is_err());
    }

    #[test]
    fn mtl_dependencies_export_png_not_raw_dds() {
        let source = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        fs::create_dir(source.path().join("characters")).unwrap();
        fs::create_dir(source.path().join("textures")).unwrap();
        let mut dds = vec![0u8; 128];
        dds[..4].copy_from_slice(b"DDS ");
        dds[4..8].copy_from_slice(&124u32.to_le_bytes());
        dds[8..12].copy_from_slice(&0x21007u32.to_le_bytes());
        dds[12..16].copy_from_slice(&4u32.to_le_bytes());
        dds[16..20].copy_from_slice(&4u32.to_le_bytes());
        dds[28..32].copy_from_slice(&1u32.to_le_bytes());
        dds[76..80].copy_from_slice(&32u32.to_le_bytes());
        dds[80..84].copy_from_slice(&4u32.to_le_bytes());
        dds[84..88].copy_from_slice(b"DXT1");
        dds.extend_from_slice(&[0, 248, 224, 7, 0, 0, 0, 0]);
        fs::write(source.path().join("textures/body.dds.0"), dds).unwrap();
        fs::write(source.path().join("characters/body.mtl"),
            br#"<Material Name="body"><Textures><Texture Map="Diffuse" File="textures/body.dds"/></Textures></Material>"#).unwrap();
        let inventory = crate::scan_directory(source.path()).unwrap();
        let (pngs, other) = dependencies(
            &fs::canonicalize(source.path()).unwrap(),
            &inventory,
            &["characters/body.mtl".to_string()],
            staging.path(),
        )
        .unwrap();
        assert_eq!((pngs, other), (1, 0));
        let png = staging.path().join("textures/textures/body.png");
        assert!(fs::read(png).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(!staging.path().join("textures/textures/body.dds.0").exists());
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
