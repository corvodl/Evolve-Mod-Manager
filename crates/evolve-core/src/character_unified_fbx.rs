//! Opt-in unified CDF character FBX, using original CryEngine skeleton/weights.
//! Fail closed on any missing part, UV, mismatched rig or invalid Blender output.
//! This is geometry/rig unification; full multi-slot materials remain a separate gate.
use crate::cdf::{AssetResolution, read_cdf, resolve_cdf, resolve_virtual};
use crate::character_assembly::select_attachments;
use crate::character_equipment::{EquipmentOverrides, validated_equipment_overrides};
use crate::fbx::publish_verified_binary_fbx;
use crate::inventory::Inventory;
use crate::materials::{material_texture_candidates, read_material, resolve_materials};
use crate::model::{original_mesh_subsets, read_model};
use crate::native_caf::read_native_caf;
use crate::rig::{
    OriginalMaterialPng, embed_original_material_slots_glb, embed_verified_material_png_glb,
    rigged_glb_bytes, rigged_glb_bytes_with_caf,
};
use image::ImageEncoder;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_SOURCE: u64 = 256 * 1024 * 1024;
const MAX_PARTS: usize = 128;

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
    if !meta.file_type().is_file()
        || meta.len() == 0
        || meta.len() > MAX_SOURCE
        || !path.starts_with(root)
    {
        return Err(format!(
            "Unsafe, unsupported or oversized source: {relative}"
        ));
    }
    Ok(path)
}

// Prefer a verified same-stem compiled skinm/chrm stream; never search by basename.
fn preferred_compiled_mesh(
    root: &Path,
    original: &Path,
    inventory: &Inventory,
) -> Result<PathBuf, String> {
    // Windows temp directories can use a different spelling/case than the
    // canonical source. Compare two canonical paths, not mixed lexical paths.
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let extension = match original.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case("skin") => "skinm",
        Some(e) if e.eq_ignore_ascii_case("chr") => "chrm",
        _ => return Ok(original.to_path_buf()),
    };
    let candidate = original.with_extension(extension);
    let relative = candidate
        .strip_prefix(&root)
        .map_err(|_| "Compiled mesh escaped original asset root")?;
    let virtual_path = relative.to_string_lossy().replace('\\', "/");
    match resolve_virtual(inventory, &virtual_path) {
        AssetResolution::Found(path) => input_file(&root, &path),
        AssetResolution::Missing => Ok(original.to_path_buf()),
        AssetResolution::Ambiguous(_) => Err("Ambiguous compiled mesh sidecar".into()),
    }
}

fn safe_part_name(raw: &str) -> Result<String, String> {
    let filtered = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if filtered.is_empty() || filtered.len() > 90 || filtered.chars().all(|c| c == '_') {
        return Err("CDF contains an invalid part name".into());
    }
    Ok(filtered)
}

type ResolvedCdfParts = (PathBuf, String, Option<String>, Vec<(String, PathBuf)>);

fn plan_parts(
    source: &Path,
    inventory: &Inventory,
    equipment: &EquipmentOverrides,
) -> Result<ResolvedCdfParts, String> {
    let root = fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
    let original = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    let actual = fs::canonicalize(source).map_err(|e| e.to_string())?;
    if !original.file_type().is_file()
        || original.len() > 16 * 1024 * 1024
        || !actual.starts_with(&root)
    {
        return Err("CDF must be a regular indexed original file".into());
    }
    let definition = read_cdf(&actual).map_err(|e| e.to_string())?;
    let validated = validated_equipment_overrides(&definition, inventory, equipment)?;
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
        let override_binding = validated.get(&part.name);
        let alternate = override_binding.map(|binding| resolve_virtual(inventory, binding));
        let resolved = alternate.as_ref().unwrap_or(resolution);
        let AssetResolution::Found(relative) = resolved else {
            return Err(format!("Part {} is missing or ambiguous", part.name));
        };
        let name = safe_part_name(&part.name)?;
        if !names.insert(name.to_ascii_lowercase()) {
            return Err("CDF has colliding attachment names".into());
        }
        let bound = input_file(&root, relative)?;
        let compiled = preferred_compiled_mesh(&root, &bound, inventory)?;
        parts.push((name, compiled));
    }
    if parts.is_empty() || parts.len() != selected.len() || parts.len() > MAX_PARTS {
        return Err("CDF has incomplete or excessive selected attachments".into());
    }
    Ok((skeleton, skeleton_ref, skin_set, parts))
}

#[derive(Default)]
struct AuthoredMaps {
    diffuse: Option<PathBuf>,
    normal: Option<PathBuf>,
    emissive: Option<PathBuf>,
    specular: Option<PathBuf>,
    gloss: Option<PathBuf>,
}

/// Resolve actual MTL texture roles, never the first same-named DDS found
/// somewhere else. Multiple submaterials need the original subset map and
/// are left untextured rather than painted with an incorrect atlas.
fn exact_single_slot_maps(
    root: &Path,
    mesh_path: &Path,
    inventory: &Inventory,
) -> Result<AuthoredMaps, String> {
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let material = mesh_path.with_extension("mtl");
    let rel = material
        .strip_prefix(&root)
        .map_err(|_| "Material path escaped the original asset root")?;
    let candidate = rel.to_string_lossy().replace('\\', "/");
    let found = match resolve_virtual(inventory, &candidate) {
        AssetResolution::Found(path) => path,
        AssetResolution::Missing => return Ok(AuthoredMaps::default()),
        AssetResolution::Ambiguous(_) => {
            return Err(format!("Ambiguous authored CDF material: {candidate}"));
        }
    };
    let document = read_material(&input_file(&root, &found)?)
        .map_err(|e| format!("Invalid authored material {candidate}: {e}"))?;
    if document.slots.len() != 1 {
        return Ok(AuthoredMaps::default());
    }
    let resolved = resolve_materials(inventory, &document);
    let mut maps = AuthoredMaps::default();
    for item in &resolved.textures {
        let target = match item.role.as_str() {
            "diffuse" => &mut maps.diffuse,
            "normal" | "normals" | "bumpmap" => &mut maps.normal,
            "emissive" | "emittance" | "emission" | "glow" => &mut maps.emissive,
            "specular" | "specularcolor" => &mut maps.specular,
            "gloss" | "glossiness" | "smoothness" => &mut maps.gloss,
            _ => continue,
        };
        if target.is_some() {
            return Err(format!(
                "Multiple original {} layers in {candidate}",
                item.role
            ));
        }
        match &item.resolution {
            AssetResolution::Found(path) => *target = Some(input_file(&root, path)?),
            AssetResolution::Missing => (),
            AssetResolution::Ambiguous(_) => {
                return Err(format!(
                    "Ambiguous authored {} in {candidate}",
                    item.authored_path
                ));
            }
        }
    }
    Ok(maps)
}

/// Resolve authored CryEngine leaf maps for every original subset, including
/// empty NoDraw leaves, without shifting indexes or borrowing a neighbor DDS.
/// Distinct PNGs stay bound to the original material ID in Blender.
fn exact_multi_slot_materials(
    root: &Path,
    mesh_path: &Path,
    inventory: &Inventory,
) -> Result<Option<Vec<OriginalMaterialPng>>, String> {
    let material = mesh_path.with_extension("mtl");
    let virtual_mtl = material
        .strip_prefix(root)
        .map_err(|_| "Original material escaped indexed asset root")?
        .to_string_lossy()
        .replace('\\', "/");
    let file = match resolve_virtual(inventory, &virtual_mtl) {
        AssetResolution::Found(file) => file,
        AssetResolution::Missing => return Ok(None),
        AssetResolution::Ambiguous(_) => return Err("Ambiguous original character MTL".into()),
    };
    let doc = read_material(&input_file(root, &file)?)
        .map_err(|e| format!("Invalid CryEngine MTL {virtual_mtl}: {e}"))?;
    if doc.slots.len() > 128 {
        return Err("CryEngine MTL exceeds 128 original slots".into());
    }
    let mut slots = Vec::with_capacity(doc.slots.len());
    for leaf in &doc.slots {
        let mut slot = OriginalMaterialPng {
            name: leaf.name.clone(),
            no_draw: leaf.shader.eq_ignore_ascii_case("NoDraw"),
            ..Default::default()
        };
        for map in &leaf.maps {
            let role = match map.role.as_str() {
                "diffuse" => 0,
                "normal" | "normals" | "bumpmap" => 1,
                "emittance" | "emissive" | "emission" | "glow" => 2,
                "specular" | "specularcolor" => 3,
                "gloss" | "glossiness" | "smoothness" => 4,
                _ => continue,
            };
            let mut found = None;
            for candidate in material_texture_candidates(&map.path) {
                match resolve_virtual(inventory, &candidate) {
                    AssetResolution::Found(path) => {
                        found = Some(input_file(root, &path)?);
                        break;
                    }
                    AssetResolution::Ambiguous(_) => {
                        return Err(format!("Ambiguous original MTL map {}", map.path));
                    }
                    AssetResolution::Missing => {}
                }
            }
            let Some(file) = found else { continue };
            let target = match role {
                0 => &mut slot.diffuse,
                1 => &mut slot.normal,
                2 => &mut slot.emissive,
                3 => &mut slot.specular,
                4 => &mut slot.roughness,
                _ => unreachable!(),
            };
            if target.is_some() {
                return Err(format!(
                    "Duplicate authored texture role in MTL slot {}",
                    leaf.name
                ));
            }
            *target = Some(if role == 4 {
                authored_roughness_png(&file)?
            } else {
                authored_png(&file, role == 1)?
            });
        }
        slots.push(slot);
    }
    Ok(Some(slots))
}

/// glTF metallicRoughnessTexture uses roughness in green and metallic in
/// blue. An explicitly authored gloss map is *inverse* roughness; never
/// derive it from arbitrary normal, specular or diffuse alpha.
fn authored_roughness_png(path: &Path) -> Result<Vec<u8>, String> {
    let (_info, mut rgba) = crate::dds::decode_dds_rgba(path)
        .map_err(|e| format!("Cannot decode authored gloss DDS {}: {e}", path.display()))?;
    for pixel in rgba.pixels_mut() {
        let roughness = 255 - pixel[0];
        pixel.0 = [0, roughness, 0, 255];
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| format!("Cannot encode authored gloss: {e}"))?;
    Ok(png)
}

fn authored_png(path: &Path, normal_map: bool) -> Result<Vec<u8>, String> {
    let (info, mut rgba) = crate::dds::decode_dds_rgba(path)
        .map_err(|e| format!("Could not decode original DDS {}: {e}", path.display()))?;
    if normal_map && info.format == crate::dds::DdsFormat::Bc5 {
        // BC5 contains tangent-space signed XY. glTF expects normal RGB;
        // reconstruct the nonnegative source Z, leaving UV0 unchanged.
        for px in rgba.pixels_mut() {
            let x = (f32::from(px[0]) / 255.0) * 2.0 - 1.0;
            let y = (f32::from(px[1]) / 255.0) * 2.0 - 1.0;
            let z = (1.0 - x * x - y * y).max(0.0).sqrt();
            px[2] = (((z + 1.0) * 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
            px[3] = 255;
        }
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| format!("Cannot encode original DDS to PNG: {e}"))?;
    Ok(png)
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
    export_unified_cdf_fbx_with_equipment(
        source,
        inventory,
        blender,
        destination,
        caf,
        &EquipmentOverrides::new(),
    )
}

/// Export the exact currently selected equipment alternatives as a verified
/// unified FBX. Unknown variants and mismatched skeletons still fail closed.
pub fn export_unified_cdf_fbx_with_equipment(
    source: &Path,
    inventory: &Inventory,
    blender: &Path,
    destination: &Path,
    caf: Option<&Path>,
    equipment: &EquipmentOverrides,
) -> Result<UnifiedCdfFbxReport, String> {
    export_unified_cdf_fbx_options(
        source, inventory, blender, destination, caf, equipment, true,
    )
}

/// Preserve authored source geometry, material slots, bone weights and UV0.
/// Toggle embedding independently of separate PNG dependency export.
pub fn export_unified_cdf_fbx_options(
    source: &Path,
    inventory: &Inventory,
    blender: &Path,
    destination: &Path,
    caf: Option<&Path>,
    equipment: &EquipmentOverrides,
    embed_textures: bool,
) -> Result<UnifiedCdfFbxReport, String> {
    let root = fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
    if !destination.is_absolute()
        || !destination
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("fbx"))
        || fs::symlink_metadata(destination).is_ok()
    {
        return Err("Choose a NEW absolute .fbx destination".into());
    }
    let parent = destination
        .parent()
        .ok_or("FBX destination has no parent")?;
    let meta = fs::symlink_metadata(parent).map_err(|e| e.to_string())?;
    let parent = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    if !meta.file_type().is_dir() || parent.starts_with(&root) {
        return Err("FBX output must be outside the original game assets".into());
    }
    let exe = fs::symlink_metadata(blender).map_err(|e| e.to_string())?;
    if !exe.file_type().is_file()
        || !blender
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
    {
        return Err("Choose an explicitly installed Blender.exe".into());
    }
    let (skeleton, skeleton_ref, skin_set, parts) = plan_parts(source, inventory, equipment)?;
    let skeleton_bytes = fs::read(&skeleton).map_err(|e| e.to_string())?;
    let motion = caf
        .map(|path| {
            let relative = path
                .strip_prefix(&root)
                .map_err(|_| "Optional CAF must be under the original indexed assets".to_owned())?;
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("caf"))
            {
                return Err("Only an original .caf can be embedded".into());
            }
            let selected = input_file(&root, &relative.to_string_lossy())?;
            read_native_caf(&selected).map_err(|e| e.to_string())
        })
        .transpose()?;

    let work = tempfile::Builder::new()
        .prefix(".evolve-unified-character-")
        .tempdir_in(&parent)
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    let mut bone_count = None;
    let mut vertices = 0usize;
    let mut total_glb_bytes = 0u64;
    let mut labels = Vec::new();
    let mut diffuse_textured_parts = 0usize;
    for (index, (name, original)) in parts.iter().enumerate() {
        let mesh = read_model(original).map_err(|e| format!("CDF part {name}: {e}"))?;
        if mesh
            .uv0
            .as_ref()
            .is_none_or(|uv| uv.len() != mesh.positions.len())
        {
            return Err(format!("CDF part {name} has no verified original UV0"));
        }
        let raw = fs::read(original).map_err(|e| e.to_string())?;
        // Embed the verified Action once, on the common skeleton. Importing
        // a duplicate CAF for every part would create competing Blender
        // actions and make the exported animation ambiguous.
        let (mut glb, verified) = match (&motion, index) {
            (Some(clip), 0) => rigged_glb_bytes_with_caf(&mesh, &raw, &skeleton_bytes, clip),
            _ => rigged_glb_bytes(&mesh, &raw, &skeleton_bytes),
        }
        .map_err(|e| format!("CDF part {name} cannot be rigged: {e}"))?;
        if bone_count.is_some_and(|count| count != verified.bone_count) {
            return Err("Compiled bone count changes between CDF parts".into());
        }
        bone_count = Some(verified.bone_count);
        vertices = vertices
            .checked_add(verified.weighted_vertices)
            .ok_or("Vertex count overflow")?;
        let source_subsets = original_mesh_subsets(&raw, mesh.indices.len())
            .map_err(|e| format!("CDF part {name} source subset mapping: {e}"))?;
        let original_materials = exact_multi_slot_materials(&root, original, inventory)?;
        // More than one source submaterial requires explicit source MTL and
        // original subset IDs; never paint every face with the first leaf.
        if source_subsets.iter().any(|(_, _, id)| *id == 255) {
            return Err(format!(
                "CDF part {name} has unresolved source material IDs"
            ));
        }
        if source_subsets.len() > 1
            || source_subsets[0].2 != 0
            || original_materials
                .as_ref()
                .is_some_and(|slots| slots.len() > 1)
        {
            let slots = original_materials.ok_or_else(|| {
                format!("CDF part {name} has several subsets but no original source MTL")
            })?;
            if source_subsets.iter().any(|(_, _, index)| *index == 255) {
                return Err(format!("CDF part {name} has an unresolved material subset"));
            }
            if slots.iter().any(|slot| slot.diffuse.is_some()) {
                diffuse_textured_parts += 1;
            }
            glb = embed_original_material_slots_glb(&glb, &source_subsets, &slots)
                .map_err(|e| format!("CDF part {name} original multi-material: {e}"))?;
        } else {
            let maps = exact_single_slot_maps(&root, original, inventory)?;
            let diffuse = maps
                .diffuse
                .as_deref()
                .map(|f| authored_png(f, false))
                .transpose()?;
            let normal = maps
                .normal
                .as_deref()
                .map(|f| authored_png(f, true))
                .transpose()?;
            let emissive = maps
                .emissive
                .as_deref()
                .map(|f| authored_png(f, false))
                .transpose()?;
            let specular = maps
                .specular
                .as_deref()
                .map(|f| authored_png(f, false))
                .transpose()?;
            let roughness = maps
                .gloss
                .as_deref()
                .map(authored_roughness_png)
                .transpose()?;
            if diffuse.is_some()
                || normal.is_some()
                || emissive.is_some()
                || specular.is_some()
                || roughness.is_some()
            {
                glb = embed_verified_material_png_glb(
                    &glb,
                    diffuse.as_deref(),
                    normal.as_deref(),
                    emissive.as_deref(),
                    specular.as_deref(),
                    roughness.as_deref(),
                )
                .map_err(|e| format!("CDF part {name} original maps: {e}"))?;
            }
            if diffuse.is_some() {
                diffuse_textured_parts += 1;
            }
        }
        total_glb_bytes = total_glb_bytes
            .checked_add(glb.len() as u64)
            .ok_or("Staged character GLB size overflow")?;
        if total_glb_bytes > 2 * 1024 * 1024 * 1024 {
            return Err("Combined character intermediates exceed 2 GiB limit".into());
        }
        let staged = work.path().join(format!("part-{index:03}.glb"));
        fs::write(&staged, &glb).map_err(|e| e.to_string())?;
        entries.push(json!({"name":name,"source":staged}));
        labels.push(name.clone());
    }
    let manifest = work.path().join("parts.json");
    fs::write(
        &manifest,
        serde_json::to_vec(&json!({
            "parts":entries, "include_animation":motion.is_some(),
            "embed_textures":embed_textures
        }))
        .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let script = work.path().join("unify.py");
    fs::write(&script, include_str!("blender_character_fbx.py")).map_err(|e| e.to_string())?;
    fs::write(work.path().join("blender_uv0.py"), include_str!("blender_uv0.py"))
        .map_err(|e| e.to_string())?;
    let temp_fbx = work.path().join("character.fbx");
    let mut child = Command::new(blender)
        .arg("--background")
        .arg("--factory-startup")
        .arg("--python-exit-code")
        .arg("1")
        .arg("--python")
        .arg(&script)
        .arg("--")
        .arg(&manifest)
        .arg(&temp_fbx)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
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
    // Shares the same byte-for-byte, non-overwriting FBX publish path as
    // single-model exports. The old flow consumed the first 23 bytes while
    // verifying the signature and accidentally discarded them on copy.
    publish_verified_binary_fbx(&temp_fbx, destination).map_err(|e| e.to_string())?;
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
    fn exact_compiled_sidecar_and_original_authored_top_mip_are_used() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("hunter.skin"), b"original binding").unwrap();
        fs::write(root.join("hunter.skinm"), b"compiled stream").unwrap();
        fs::create_dir(root.join("textures")).unwrap();
        fs::write(root.join("textures/hunter.dds"), b"lower mip").unwrap();
        fs::write(root.join("textures/hunter.dds.0"), b"highest mip").unwrap();
        fs::write(
            root.join("hunter.mtl"),
            br#"<Material Name="Hunter"><Textures><Texture Map="Diffuse" File="textures/hunter.tif"/></Textures></Material>"#,
        ).unwrap();
        let inventory = crate::scan_directory(root).unwrap();
        let original = fs::canonicalize(root.join("hunter.skin")).unwrap();
        let compiled = preferred_compiled_mesh(root, &original, &inventory).unwrap();
        assert!(compiled.ends_with("hunter.skinm"));
        let diffuse = exact_single_slot_maps(root, &compiled, &inventory)
            .unwrap()
            .diffuse
            .unwrap();
        assert!(diffuse.ends_with("hunter.dds.0"));
        assert_eq!(fs::read(original).unwrap(), b"original binding");
    }

    #[test]
    fn multiple_material_slots_never_become_a_guessed_single_texture() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("hunter.skinm"), b"original stream").unwrap();
        fs::write(root.join("hunter.mtl"),
            br#"<Materials><Material Name="body"><Textures><Texture Map="Diffuse" File="textures/body.dds"/></Textures></Material><Material Name="suit"><Textures><Texture Map="Diffuse" File="textures/suit.dds"/></Textures></Material></Materials>"#
        ).unwrap();
        let inventory = crate::scan_directory(root).unwrap();
        let mesh = fs::canonicalize(root.join("hunter.skinm")).unwrap();
        assert!(
            exact_single_slot_maps(root, &mesh, &inventory)
                .unwrap()
                .diffuse
                .is_none()
        );
    }

    #[test]
    fn missing_original_cdf_never_produces_export() {
        let root = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let cdf = root.path().join("character.cdf");
        fs::write(&cdf, br#"<CharacterDefinition><Model File="missing.chr"/><AttachmentList><Attachment Type="CA_SKIN" AName="Body" Binding="missing.skin"/></AttachmentList></CharacterDefinition>"#).unwrap();
        let inv = crate::scan_directory(root.path()).unwrap();
        assert!(plan_parts(&cdf, &inv, &EquipmentOverrides::new()).is_err());
        let target = output.path().join("character.fbx");
        assert!(
            export_unified_cdf_fbx(&cdf, &inv, Path::new("Blender.exe"), &target, None).is_err()
        );
        assert!(!target.exists());
    }
}
