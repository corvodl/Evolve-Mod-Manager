//! Atomic, flat export of a CDF's verified skinned attachments.
//!
//! This is an intermediate folder workflow: separate per-attachment FBXs,
//! **not** the private manager's unified root character FBX. Nothing is
//! published if a single native mesh or skeleton fails verification.
use crate::cdf::{AssetResolution, read_cdf, resolve_cdf};
use crate::character_assembly::select_attachments;
use crate::inventory::Inventory;
use crate::rig::export_rigged_character_fbx_textured_animated;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlatCdfFbxReport {
    pub files: Vec<String>,
    pub original_skeleton: String,
    pub selected_skin_set: Option<String>,
    pub animations_embedded: bool,
}

/// No paths from a CDF, inventory or symlink may escape its verified input root.
fn verified_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let candidate = root.join(relative);
    let meta = fs::symlink_metadata(&candidate).map_err(|e| e.to_string())?;
    let actual = fs::canonicalize(candidate).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || !actual.starts_with(root) {
        return Err(format!("Unsafe or missing source asset: {relative}"));
    }
    Ok(actual)
}

fn safe_output_name(label: &str) -> Result<String, String> {
    let stem = label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if stem.is_empty() || stem.len() > 90 || stem.chars().all(|c| c == '_') {
        return Err("Invalid CDF part name".into());
    }
    Ok(format!("{stem}.fbx"))
}

/// Export every selected CDF attachment into one new destination folder, with
/// no nested assembly/animation subdirectories. Each source mesh must support
/// independently verified native weights. An optional exact CAF is embedded
/// in each part only when its bone CRCs match the same original skeleton.
///
/// Multi-slot materials, complete character-level material mapping, and
/// unified root FBX rigs are not yet supported. Never advertise this output
/// as the completed production character exporter.
pub fn export_flat_cdf_parts_fbx(
    source: &Path,
    inventory: &Inventory,
    blender: &Path,
    destination: &Path,
    caf: Option<&Path>,
) -> Result<FlatCdfFbxReport, String> {
    let root = fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
    if !destination.is_absolute() || fs::symlink_metadata(destination).is_ok() {
        return Err("Choose a NEW absolute export directory".into());
    }
    let parent = destination.parent().ok_or("No export parent directory")?;
    let parent_meta = fs::symlink_metadata(parent).map_err(|e| e.to_string())?;
    let export_parent = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    if !parent_meta.file_type().is_dir() || export_parent.starts_with(&root) {
        return Err("Export destination cannot be inside the indexed game assets".into());
    }
    let cdf_meta = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    let cdf_path = fs::canonicalize(source).map_err(|e| e.to_string())?;
    if !cdf_meta.file_type().is_file() || !cdf_path.starts_with(&root) {
        return Err("CDF must be an indexed regular file".into());
    }
    let definition = read_cdf(&cdf_path).map_err(|e| e.to_string())?;
    let (skin_set, selected) = select_attachments(&definition).map_err(|e| e.to_string())?;
    let refs = resolve_cdf(inventory, &definition);
    let AssetResolution::Found(skeleton_relative) = refs.skeleton else {
        return Err("Original skeleton is missing or ambiguous".into());
    };
    let skeleton = verified_file(&root, &skeleton_relative)?;
    let caf = caf.map(|path| {
        let relative = path.strip_prefix(&root)
            .map_err(|_| "CAF must be from the indexed original asset root".to_owned())?;
        verified_file(&root, &relative.to_string_lossy())
    }).transpose()?;
    if let Some(ref clip) = caf {
        if !clip.extension().is_some_and(|v| v.eq_ignore_ascii_case("caf")) {
            return Err("Only verified original CAF Actions are supported".into());
        }
    }
    let mut attachments = Vec::new();
    let mut names = HashSet::new();
    for (part, (_, resolution)) in definition.attachments.iter().zip(&refs.attachments) {
        if !selected.contains(&part.name.as_str()) {
            continue;
        }
        let AssetResolution::Found(relative) = resolution else {
            return Err(format!("CDF part {} is missing or ambiguous", part.name));
        };
        let file = verified_file(&root, relative)?;
        let filename = safe_output_name(&part.name)?;
        if !names.insert(filename.to_ascii_lowercase()) {
            return Err("CDF output would contain duplicate part names".into());
        }
        attachments.push((filename, file));
    }
    if attachments.len() != selected.len() || attachments.is_empty() {
        return Err("Not all requested CDF parts resolved uniquely".into());
    }
    let stage = tempfile::Builder::new()
        .prefix(".evolve-flat-cdf-")
        .tempdir_in(&export_parent)
        .map_err(|e| e.to_string())?;
    let mut exported = Vec::new();
    for (filename, mesh) in attachments {
        let output = stage.path().join(&filename);
        export_rigged_character_fbx_textured_animated(
            &mesh, &skeleton, blender, None, caf.as_deref(), &output,
        ).map_err(|e| format!("CDF part {filename} cannot be exported: {e}"))?;
        exported.push(filename);
    }
    // A complete stage is made visible only after every part verifies.
    let stage_path = stage.keep();
    if fs::symlink_metadata(destination).is_ok() {
        let _ = fs::remove_dir_all(&stage_path);
        return Err("Export destination was created by another process".into());
    }
    if let Err(error) = fs::rename(&stage_path, destination) {
        let _ = fs::remove_dir_all(&stage_path);
        return Err(format!("Unable to publish complete character export: {error}"));
    }
    Ok(FlatCdfFbxReport {
        files: exported,
        original_skeleton: skeleton_relative,
        selected_skin_set: skin_set,
        animations_embedded: caf.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_collisions_and_invalid_part_names_are_rejected() {
        assert_eq!(safe_output_name("Body Skin").unwrap(), "Body_Skin.fbx");
        assert!(safe_output_name("...").is_err());
        assert!(safe_output_name("").is_err());
    }

    #[test]
    fn incomplete_cdf_creates_no_output_directory() {
        let input = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let cdf = input.path().join("body.cdf");
        fs::write(&cdf, br#"<CharacterDefinition><Model File="characters/missing.chr"/><AttachmentList><Attachment Type="CA_SKIN" AName="Body" Binding="characters/missing.skin"/></AttachmentList></CharacterDefinition>"#).unwrap();
        let inventory = crate::scan_directory(input.path()).unwrap();
        let destination = output.path().join("character");
        assert!(export_flat_cdf_parts_fbx(
            &cdf, &inventory, Path::new("Blender.exe"), &destination, None
        ).is_err());
        assert!(!destination.exists());
    }

    #[test]
    fn export_refuses_to_write_inside_originals() {
        let input = tempfile::tempdir().unwrap();
        let cdf = input.path().join("body.cdf");
        fs::write(&cdf, b"invalid").unwrap();
        let inventory = crate::scan_directory(input.path()).unwrap();
        let target = input.path().join("exported");
        assert!(export_flat_cdf_parts_fbx(
            &cdf, &inventory, Path::new("Blender.exe"), &target, None
        ).is_err());
        assert!(!target.exists());
    }
}
