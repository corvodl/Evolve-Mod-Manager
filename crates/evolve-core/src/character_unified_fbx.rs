//! Opt-in unified CDF character FBX, using original CryEngine skeleton/weights.
//! Fail closed on any missing part, UV, mismatched rig or invalid Blender output.
//! This is geometry/rig unification; full multi-slot materials remain a separate gate.
use crate::cdf::{AssetResolution, read_cdf, resolve_cdf, resolve_virtual};
use crate::character_assembly::select_attachments;
use crate::inventory::Inventory;
use crate::model::read_model;
use crate::materials::{read_material, resolve_materials};
use crate::native_caf::read_native_caf;
use crate::rig::{embed_diffuse_png_glb, rigged_glb_bytes, rigged_glb_bytes_with_caf};
use image::ImageEncoder;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_SOURCE: u64 = 256 * 1024 * 1024;
const MAX_PARTS: usize = 128;
const FBX_HEADER: &[u8] = b"Kaydara FBX Binary  \x00\x1a\x00";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedCdfFbxReport {
    pub original_skeleton: String,
    pub selected_skin_set: Option<String>,
    pub parts: Vec<String>,
    pub bones: usize,
    pub weighted_vertices: usize,
    pub diffuse_textured_parts: usize,
    pub animations_embedded: bool,
    /// These are not yet a complete set of original per-submaterial textures.
    pub fully_textured: bool,
}

fn input_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let target = root.join(relative);
    let meta = fs::symlink_metadata(&target).map_err(|e| e.to_string())?;
    let path = fs::canonicalize(target).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() == 0
        || meta.len() > MAX_SOURCE || !path.starts_with(root)
    {
        return Err(format!("Unsafe, unsupported or oversized source: {relative}"));
    }
    Ok(path)
}

fn safe_part_name(raw: &str) -> Result<String, String> {
    let filtered = raw
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') { ch } else { '_' })
        .collect::<String>();
    if filtered.is_empty() || filtered.len() > 90 || filtered.chars().all(|c| c == '_') {
        return Err("CDF contains an invalid part name".into());
    }
    Ok(filtered)
}

fn plan_parts(source: &Path, inventory: &Inventory) -> Result<(PathBuf, String, Option<String>, Vec<(String, PathBuf)>), String> {
    let root = fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
    let original = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    let actual = fs::canonicalize(source).map_err(|e| e.to_string())?;
    if !original.file_type().is_file() || original.len() > 16 * 1024 * 1024
        || !actual.starts_with(&root)
    {
        return Err("CDF must be a regular indexed original file".into());
    }
    let definition = read_cdf(&actual).map_err(|e| e.to_string())?;
    let (skin_set, selected) = select_attachments(&definition).map_err(|e| e.to_string())?;
    let resolved = resolve_cdf(inventory, &definition);
    let AssetResolution::Found(skeleton_ref) = resolved.skeleton else {
        return Err("Original CDF skeleton is missing or ambiguous".into());
    };
    let skeleton = input_file(&root, &skeleton_ref)?;
    let mut parts = Vec::new();
    let mut names = HashSet::new();
    for (part, (_, resolution)) in definition.attachments.iter().zip(&resolved.attachments) {
        if !selected.contains(&part.name.as_str()) {
            continue;
        }
        let AssetResolution::Found(relative) = resolution else {
            return Err(format!("Part {} is missing or ambiguous", part.name));
        };
        let name = safe_part_name(&part.name)?;
        if !names.insert(name.to_ascii_lowercase()) {
            return Err("CDF has colliding attachment names".into());
        }
        parts.push((name, input_file(&root, relative)?));
    }
    if parts.is_empty() || parts.len() != selected.len() || parts.len() > MAX_PARTS {
        return Err("CDF has incomplete or excessive selected attachments".into());
    }
    Ok((skeleton, skeleton_ref, skin_set, parts))
}

/// Only use one authored diffuse map in an unambiguous single-slot material.
/// Multiple submaterials and authored overrides are intentionally not guessed.
fn exact_single_slot_diffuse(
    root: &Path,
    mesh_path: &Path,
    inventory: &Inventory,
) -> Result<Option<PathBuf>, String> {
    let material = mesh_path.with_extension("mtl");
    let rel = material.strip_prefix(root)
        .map_err(|_| "Material path escaped the original asset root")?;
    let candidate = rel.to_string_lossy().replace('\\', "/");
    let found = match resolve_virtual(inventory, &candidate) {
        AssetResolution::Found(path) => path,
        AssetResolution::Missing => return Ok(None),
        AssetResolution::Ambiguous(_) => {
            return Err(format!("Ambiguous authored CDF material: {candidate}"));
        }
    };
    let document = read_material(&input_file(root, &found)?)
        .map_err(|e| format!("Invalid authored material {candidate}: {e}"))?;
    if document.slots.len() != 1 {
        return Ok(None);
    }
    let resolved = resolve_materials(inventory, &document);
    let diffuse = resolved.textures.iter()
        .filter(|t| t.role == "diffuse").collect::<Vec<_>>();
    if diffuse.is_empty() {
        return Ok(None);
    }
    if diffuse.len() != 1 {
        return Err(format!("Multiple authored diffuse textures in {candidate}"));
    }
    match &diffuse[0].resolution {
        AssetResolution::Found(path) => input_file(root, path).map(Some),
        AssetResolution::Missing => Ok(None),
        AssetResolution::Ambiguous(_) => Err(format!(
            "Ambiguous authored diffuse {} in {candidate}", diffuse[0].authored_path
        )),
    }
}

/// Make ONE armature-bound FBX from original CDF attachments only when every
/// decoded mesh has verified native skeleton weights and original UV0.
/// A separate, explicitly selected compatible CAF is opt-in. No automatic
/// DBA action collection or material slot reconstruction is claimed.
pub fn export_unified_cdf_fbx(
    source: &Path,
    inventory: &Inventory,
    blender: &Path,
    destination: &Path,
    caf: Option<&Path>,
) -> Result<UnifiedCdfFbxReport, String> {
    let root = fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
    if !destination.is_absolute()
        || !destination.extension().is_some_and(|e| e.eq_ignore_ascii_case("fbx"))
        || fs::symlink_metadata(destination).is_ok()
    {
        return Err("Choose a NEW absolute .fbx destination".into());
    }
    let parent = destination.parent().ok_or("FBX destination has no parent")?;
    let meta = fs::symlink_metadata(parent).map_err(|e| e.to_string())?;
    let parent = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    if !meta.file_type().is_dir() || parent.starts_with(&root) {
        return Err("FBX output must be outside the original game assets".into());
    }
    let exe = fs::symlink_metadata(blender).map_err(|e| e.to_string())?;
    if !exe.file_type().is_file()
        || !blender.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))
    {
        return Err("Choose an explicitly installed Blender.exe".into());
    }
    let (skeleton, skeleton_ref, skin_set, parts) = plan_parts(source, inventory)?;
    let skeleton_bytes = fs::read(&skeleton).map_err(|e| e.to_string())?;
    let motion = caf.map(|path| {
        let relative = path.strip_prefix(&root)
            .map_err(|_| "Optional CAF must be under the original indexed assets".to_owned())?;
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("caf")) {
            return Err("Only an original .caf can be embedded".into());
        }
        let selected = input_file(&root, &relative.to_string_lossy())?;
        read_native_caf(&selected).map_err(|e| e.to_string())
    }).transpose()?;

    let work = tempfile::Builder::new()
        .prefix(".evolve-unified-character-")
        .tempdir_in(&parent)
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    let mut bone_count = None;
    let mut vertices = 0usize;
    let mut labels = Vec::new();
    let mut diffuse_textured_parts = 0usize;
    for (index, (name, original)) in parts.iter().enumerate() {
        let mesh = read_model(original).map_err(|e| format!("CDF part {name}: {e}"))?;
        if mesh.uv0.as_ref().is_none_or(|uv| uv.len() != mesh.positions.len()) {
            return Err(format!("CDF part {name} has no verified original UV0"));
        }
        let raw = fs::read(original).map_err(|e| e.to_string())?;
        let (mut glb, verified) = match &motion {
            Some(clip) => rigged_glb_bytes_with_caf(&mesh, &raw, &skeleton_bytes, clip),
            None => rigged_glb_bytes(&mesh, &raw, &skeleton_bytes),
        }.map_err(|e| format!("CDF part {name} cannot be rigged: {e}"))?;
        if bone_count.is_some_and(|count| count != verified.bone_count) {
            return Err("Compiled bone count changes between CDF parts".into());
        }
        bone_count = Some(verified.bone_count);
        vertices = vertices.checked_add(verified.weighted_vertices)
            .ok_or("Vertex count overflow")?;
        if let Some(dds_path) = exact_single_slot_diffuse(&root, original, inventory)? {
            let (_info, rgba) = crate::dds::decode_dds_rgba(&dds_path)
                .map_err(|e| format!("CDF part {name}: authored DDS decode: {e}"))?;
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(
                    rgba.as_raw(), rgba.width(), rgba.height(),
                    image::ExtendedColorType::Rgba8,
                ).map_err(|e| format!("CDF part {name}: PNG encoding: {e}"))?;
            glb = embed_diffuse_png_glb(&glb, &png).map_err(|e| e.to_string())?;
            diffuse_textured_parts += 1;
        }
        let staged = work.path().join(format!("part-{index:03}.glb"));
        fs::write(&staged, &glb).map_err(|e| e.to_string())?;
        entries.push(json!({"name":name,"source":staged}));
        labels.push(name.clone());
    }
    let manifest = work.path().join("parts.json");
    fs::write(&manifest, serde_json::to_vec(&json!({
        "parts":entries, "include_animation":motion.is_some()
    })).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let script = work.path().join("unify.py");
    fs::write(&script, include_str!("blender_character_fbx.py"))
        .map_err(|e| e.to_string())?;
    let temp_fbx = work.path().join("character.fbx");
    let mut child = Command::new(blender)
        .arg("--background").arg("--factory-startup")
        .arg("--python").arg(&script).arg("--")
        .arg(&manifest).arg(&temp_fbx)
        .stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) if status.success() => break,
            Some(status) => return Err(format!("Blender character validation failed: {status}")),
            None if start.elapsed() > Duration::from_secs(1800) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Blender character export timed out".into());
            }
            None => std::thread::sleep(Duration::from_millis(150)),
        }
    }
    let mut file = File::open(&temp_fbx).map_err(|e| e.to_string())?;
    let mut header = [0u8; 23];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if header != FBX_HEADER || file.metadata().map_err(|e| e.to_string())?.len() < 256 {
        return Err("Blender did not produce a verified binary FBX".into());
    }
    let mut publish = tempfile::Builder::new().prefix(".fbx-validated-")
        .tempfile_in(&parent).map_err(|e| e.to_string())?;
    std::io::copy(&mut file, &mut publish).map_err(|e| e.to_string())?;
    publish.as_file_mut().sync_all().map_err(|e| e.to_string())?;
    publish.persist_noclobber(destination).map_err(|e| e.error.to_string())?;
    Ok(UnifiedCdfFbxReport {
        original_skeleton: skeleton_ref,
        selected_skin_set: skin_set,
        parts: labels,
        bones: bone_count.unwrap_or(0),
        weighted_vertices: vertices,
        diffuse_textured_parts,
        animations_embedded: motion.is_some(),
        fully_textured: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_escaping_parts_are_rejected() {
        assert!(safe_part_name("...").is_err());
        assert!(safe_part_name("").is_err());
        assert_eq!(safe_part_name("Body Skin").unwrap(), "Body_Skin");
    }

    #[test]
    fn missing_original_cdf_never_produces_export() {
        let root = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let cdf = root.path().join("character.cdf");
        fs::write(&cdf, br#"<CharacterDefinition><Model File="missing.chr"/><AttachmentList><Attachment Type="CA_SKIN" AName="Body" Binding="missing.skin"/></AttachmentList></CharacterDefinition>"#).unwrap();
        let inv = crate::scan_directory(root.path()).unwrap();
        assert!(plan_parts(&cdf, &inv).is_err());
        let target = output.path().join("character.fbx");
        assert!(export_unified_cdf_fbx(&cdf, &inv, Path::new("Blender.exe"), &target, None).is_err());
        assert!(!target.exists());
    }
}
