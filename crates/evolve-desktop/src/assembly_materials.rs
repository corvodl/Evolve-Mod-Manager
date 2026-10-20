//! Resolve each CDF attachment's original CryEngine material and cooked DDS
//! independently. All reads are source/path-checked, background-only, and
//! shared through the bounded decoded preview cache.
use crate::asset_cache::{SharedCache, decoded_dds_indexed};
use crate::auto_resources;
use crate::textured::{MAX_GPU_PREVIEW_TEXTURE_BYTES, Surface};
use evolve_core::{
    AssemblyReport, Inventory, MaterialDocument, material_texture_candidates, read_material,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_DECODED_ASSEMBLY_BYTES: usize = 768 * 1024 * 1024;

/// MtlName 0x802 is an explicit author-authored CryEngine reference.
/// This is more reliable than guessing an equipment texture from its filename.
fn embedded_material_hints(
    binding: &str,
    inventory: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<Vec<String>, String> {
    let rendering = match binding.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("skin") => format!("{stem}.skinm"),
        Some((stem, ext)) if ext.eq_ignore_ascii_case("chr") => format!("{stem}.chrm"),
        _ => binding.to_owned(),
    };
    let path = auto_resources::exact(inventory, &rendering, imported, build)?
        .or(auto_resources::exact(inventory, binding, imported, build)?);
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() > 16 * 1024 * 1024 {
        return Ok(Vec::new());
    }
    let data = std::fs::read(&path).map_err(|e| e.to_string())?;
    if data.len() < 32 || data.get(..5) != Some(b"CrChF") {
        return Ok(Vec::new());
    }
    let u32_at = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
    };
    let Some(count) = u32_at(8).map(|v| v as usize) else {
        return Ok(Vec::new());
    };
    let Some(start) = u32_at(12).map(|v| v as usize) else {
        return Ok(Vec::new());
    };
    if count == 0
        || count > 10000
        || start < 16
        || start
            .checked_add(count * 16)
            .is_none_or(|end| end > data.len())
    {
        return Ok(Vec::new());
    }
    let mut hints = Vec::new();
    for i in 0..count {
        let at = start + i * 16;
        if u32_at(at) != Some(0x08021014) {
            continue;
        }
        let (Some(len), Some(offset)) = (u32_at(at + 8), u32_at(at + 12)) else {
            continue;
        };
        let (len, offset) = (len as usize, offset as usize);
        if len < 132
            || offset < start + count * 16
            || offset.checked_add(len).is_none_or(|end| end > data.len())
        {
            continue;
        }
        let Some(field) = data.get(offset..offset + 128) else {
            continue;
        };
        let Some(zero) = field.iter().position(|b| *b == 0) else {
            continue;
        };
        let Ok(name) = std::str::from_utf8(&field[..zero]) else {
            continue;
        };
        let hint = name.trim().replace('\\', "/");
        if hint.is_empty() {
            continue;
        }
        let hint = if hint.to_ascii_lowercase().ends_with(".mtl") {
            hint
        } else {
            format!("{hint}.mtl")
        };
        if !hints
            .iter()
            .any(|value: &String| value.eq_ignore_ascii_case(&hint))
        {
            hints.push(hint);
        }
    }
    Ok(hints)
}

/// A 2012 CryTek 0x745 character part can be "rocky3_body.chr" while
/// the source material is the one authored for its containing Rocky folder.
/// Only that exact owning-directory MTL is considered, only when the source
/// file is verified historical geometry, and never for Stage 2 skins.
fn legacy_owner_material(
    binding: &str,
    inventory: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let parent = binding.rsplit_once('/').map_or("", |(parent, _)| parent);
    let folder = parent.rsplit('/').next().unwrap_or("");
    let stem = binding
        .rsplit('/')
        .next()
        .unwrap_or(binding)
        .rsplit_once('.')
        .map_or(binding, |(stem, _)| stem);
    if folder.len() < 4
        || !stem
            .to_ascii_lowercase()
            .starts_with(&folder.to_ascii_lowercase())
    {
        return Ok(None);
    }
    let suffix = &stem[folder.len()..];
    if !suffix.is_empty() && !suffix.starts_with('_') && !suffix.as_bytes()[0].is_ascii_digit() {
        return Ok(None);
    }
    let Some(source) = auto_resources::exact(inventory, binding, imported, build)? else {
        return Ok(None);
    };
    use std::io::Read;
    let mut header = [0u8; 16];
    let mut file = std::fs::File::open(source).map_err(|error| error.to_string())?;
    if file.read_exact(&mut header).is_err()
        || &header[..8] != b"CryTek\0\0"
        || u32::from_le_bytes(header[12..16].try_into().unwrap_or_default()) != 0x745
    {
        return Ok(None);
    }
    auto_resources::exact(
        inventory,
        &format!("{parent}/{folder}.mtl"),
        imported,
        build,
    )
}

pub(crate) fn material_for_part(
    binding: &str,
    explicit: Option<&str>,
    inventory: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    if let Some(explicit) = explicit {
        // Some CDFs use a relative material basename. Try the owning
        // attachment folder first; then the complete virtual path.
        let parent = binding.rsplit_once('/').map_or("", |(folder, _)| folder);
        let root_path = explicit.starts_with("characters/")
            || explicit.starts_with("objects/")
            || explicit.starts_with("materials/");
        if !root_path && !parent.is_empty() {
            let scoped = format!("{parent}/{explicit}");
            if let Some(found) = auto_resources::exact(inventory, &scoped, imported, build)? {
                return Ok(Some(found));
            }
        }
        return auto_resources::exact(inventory, explicit, imported, build)?
            .ok_or_else(|| format!("Source CDF material not found: {explicit}"))
            .map(Some);
    }
    // Compiled MtlName data is authoritative before a fallback same-stem
    // material; multiple contradictory authored hints remain ambiguous.
    let parent = binding.rsplit_once('/').map_or("", |(folder, _)| folder);
    let embedded = embedded_material_hints(binding, inventory, imported, build)?;
    let mut unique = Vec::new();
    for hint in embedded {
        let mut paths = vec![hint.clone()];
        if !hint.starts_with("characters/")
            && !hint.starts_with("objects/")
            && !hint.starts_with("materials/")
            && !parent.is_empty()
        {
            paths.insert(0, format!("{parent}/{hint}"));
        }
        for path in paths {
            if let Some(found) = auto_resources::exact(inventory, &path, imported, build)?
                && !unique.contains(&found)
            {
                unique.push(found);
            }
        }
    }
    if unique.len() > 1 {
        return Err(format!(
            "Conflicting compiled material references: {binding}"
        ));
    }
    if let Some(found) = unique.pop() {
        return Ok(Some(found));
    }
    // Only a same-stem material is considered, never a random DDS or a
    // different character's material from another shared archive.
    let stem = binding
        .rsplit('/')
        .next()
        .unwrap_or(binding)
        .rsplit_once('.')
        .map_or(binding, |(name, _)| name);
    let mut bases = vec![stem.to_owned()];
    let lower = stem.to_ascii_lowercase();
    if let Some(at) = lower.rfind("_lod")
        && lower[at + 4..].bytes().all(|c| c.is_ascii_digit())
    {
        bases.push(stem[..at].to_owned());
    }
    for base in bases {
        let path = if parent.is_empty() {
            format!("{base}.mtl")
        } else {
            format!("{parent}/{base}.mtl")
        };
        if let Some(found) = auto_resources::exact(inventory, &path, imported, build)? {
            return Ok(Some(found));
        }
    }
    legacy_owner_material(binding, inventory, imported, build)
}

#[derive(Default)]
struct TextureBudget {
    bytes_loaded: usize,
    decoded: HashSet<PathBuf>,
}

fn role_channel(role: &str) -> Option<usize> {
    match role {
        "diffuse" => Some(0),
        "normals" | "normal" | "bumpmap" => Some(1),
        "emittance" | "emissive" | "emission" | "glow" => Some(2),
        "specular" | "specularcolor" => Some(3),
        "gloss" | "glossiness" | "smoothness" => Some(4),
        "opacity" => Some(5),
        _ => None,
    }
}

/// Per-channel resolved source textures and independent missing-map diagnostics.
type ResolvedMaterialMaps = ([Option<Arc<image::RgbaImage>>; 6], Vec<String>);

fn authored_maps(
    document: &MaterialDocument,
    material_slot: u32,
    inventory: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
    cache: &SharedCache,
    budget: &mut TextureBudget,
) -> Result<ResolvedMaterialMaps, String> {
    let slot = document.slots.get(material_slot as usize).ok_or_else(|| {
        format!(
            "Source material slot {} exceeds {} authored leaves",
            material_slot,
            document.slots.len()
        )
    })?;
    let mut result: [Option<Arc<image::RgbaImage>>; 6] = std::array::from_fn(|_| None);
    let mut warnings = Vec::new();
    // A failed opacity/normal/gloss lookup must not discard a separately
    // verified diffuse, nor may duplicate candidate roles pick an arbitrary
    // layer based on XML ordering.
    let mut counts = [0usize; 6];
    for map in &slot.maps {
        if let Some(index) = role_channel(&map.role) {
            counts[index] += 1;
        }
    }
    for map in &slot.maps {
        let Some(index) = role_channel(&map.role) else {
            continue;
        };
        if counts[index] > 1 {
            if !warnings
                .iter()
                .any(|w: &String| w.contains(&format!("channel {index}")))
            {
                warnings.push(format!(
                    "Ambiguous authored channel {index}: multiple maps for {}; left unassigned",
                    map.role
                ));
            }
            continue;
        }
        let decoded = (|| -> Result<(PathBuf, Arc<image::RgbaImage>), String> {
            let mut found = None;
            for candidate in material_texture_candidates(&map.path) {
                if let Some(path) = auto_resources::exact(inventory, &candidate, imported, build)? {
                    found = Some((path, candidate));
                    break;
                }
            }
            let (path, virtual_path) =
                found.ok_or_else(|| format!("Missing exact cooked DDS for {}", map.path))?;
            let canonical = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
            let (_info, image) =
                decoded_dds_indexed(cache, &canonical, &virtual_path, inventory, imported, build)?;
            Ok((canonical, image))
        })();
        let (canonical, image) = match decoded {
            Ok(decoded) => decoded,
            Err(error) => {
                warnings.push(format!("{}: {error}", map.role));
                continue;
            }
        };
        if !budget.decoded.contains(&canonical) {
            budget.bytes_loaded = budget
                .bytes_loaded
                .checked_add(image.as_raw().len())
                .ok_or("Decoded CDF texture budget overflow")?;
            if budget.bytes_loaded > MAX_DECODED_ASSEMBLY_BYTES {
                return Err("CDF texture budget exceeded (768 MiB)".into());
            }
            budget.decoded.insert(canonical);
        }
        result[index] = Some(image);
    }
    Ok((result, warnings))
}

/// The RGB source DDS alpha is *not* automatically an opacity channel:
/// CryEngine materials can store gloss or mask data there. Blend only for
/// explicit transparency; a non-opacity body slot stays depth-opaque.
fn is_translucent_shader(slot: &evolve_core::MaterialSlot) -> bool {
    matches!(slot.shader.to_ascii_lowercase().as_str(), "glass" | "water")
}

fn needs_alpha_blending(slot: &evolve_core::MaterialSlot) -> bool {
    is_translucent_shader(slot) || slot.maps.iter().any(|map| map.role == "opacity")
}

/// Load exact source subset/material IDs for a standalone .skinm/.chrm.
/// The ordinary model viewer previously resolved an MTL's textures but never
/// built the per-subset GPU surfaces, so 11-slot models appeared untextured.
pub fn load_model(
    source: &Path,
    material: &Path,
    inventory: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
    cache: &SharedCache,
) -> Result<(Vec<Surface>, Vec<String>), String> {
    let raw = std::fs::read(source).map_err(|e| e.to_string())?;
    let mesh = evolve_core::parse_model_bytes(&raw).map_err(|e| e.to_string())?;
    let document = read_material(material).map_err(|e| e.to_string())?;
    let ranges =
        evolve_core::original_mesh_subsets(&raw, mesh.indices.len()).map_err(|e| e.to_string())?;
    let has_table = evolve_core::has_original_mesh_subset_table(&raw).map_err(|e| e.to_string())?;
    if !has_table && document.slots.len() != 1 {
        return Err(format!(
            "{} authored materials but no original mesh-subset table. Select the exact CDF or a compiled render model with verified subset IDs; no guessed textures applied",
            document.slots.len()
        ));
    }
    let mut result = Vec::with_capacity(ranges.len());
    let mut warnings = Vec::new();
    let mut budget = TextureBudget::default();
    for (first_index, index_count, material_slot) in ranges {
        let mut surface = Surface {
            first_index,
            index_count,
            visible: true,
            transparent: false,
            diffuse: None,
            normal: None,
            emissive: None,
            specular: None,
            gloss: None,
            opacity: None,
        };
        if material_slot == 255 {
            warnings.push(format!(
                "Unassigned original face range {}..{} remains untextured",
                first_index,
                first_index + index_count
            ));
        } else {
            let leaf = document.slots.get(material_slot as usize).ok_or_else(|| {
                format!(
                    "Original mesh subset references slot {} but the MTL has only {} leaves",
                    material_slot,
                    document.slots.len()
                )
            })?;
            if leaf.shader.eq_ignore_ascii_case("NoDraw") {
                surface.visible = false;
            } else {
                match authored_maps(
                    &document,
                    material_slot,
                    inventory,
                    imported,
                    build,
                    cache,
                    &mut budget,
                ) {
                    Ok(([diffuse, normal, emissive, specular, gloss, opacity], map_warnings)) => {
                        warnings.extend(map_warnings.into_iter().map(|w| {
                            format!(
                                "Original material slot {} ({}): {w}",
                                material_slot, leaf.name
                            )
                        }));
                        surface.diffuse = diffuse;
                        surface.normal = normal;
                        surface.emissive = emissive;
                        surface.specular = specular;
                        surface.gloss = gloss;
                        surface.opacity = opacity;
                        // Only an actual verified opacity map may activate its
                        // transparency path; a missing mask must not expose a
                        // completely unrelated diffuse alpha channel.
                        surface.transparent = needs_alpha_blending(leaf)
                            && (is_translucent_shader(leaf) || surface.opacity.is_some());
                    }
                    Err(error) => warnings.push(format!(
                        "Original material slot {} ({}): {}",
                        material_slot, leaf.name, error
                    )),
                }
            }
        }
        result.push(surface);
    }
    warn_gpu_material_limit(&result, &mut warnings);
    Ok((result, warnings))
}

/// Estimate the same unique GPU role/source uploads used by the renderer.
/// A CDF with repeated sections must not count one cached DDS repeatedly.
fn unique_gpu_material_bytes(surfaces: &[Surface]) -> usize {
    let mut seen = HashSet::new();
    let mut bytes = 0usize;
    for surface in surfaces.iter().filter(|surface| surface.visible) {
        for (role, image) in [
            &surface.diffuse,
            &surface.normal,
            &surface.emissive,
            &surface.specular,
            &surface.gloss,
            &surface.opacity,
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(image) = image
                && seen.insert((Arc::as_ptr(image) as usize, role))
            {
                bytes = bytes.saturating_add(image.as_raw().len());
            }
        }
    }
    bytes
}

fn warn_gpu_material_limit(surfaces: &[Surface], warnings: &mut Vec<String>) {
    if unique_gpu_material_bytes(surfaces) > MAX_GPU_PREVIEW_TEXTURE_BYTES {
        warnings.push(
            "GPU texture preview exceeds the 256 MiB upload limit. Some maps use neutral previews to avoid a renderer crash; the original source files are untouched.".into(),
        );
    }
}

pub fn load(
    report: &AssemblyReport,
    inventory: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
    cache: &SharedCache,
) -> (Vec<Surface>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut surfaces = Vec::with_capacity(report.source_parts.len());
    let mut budget = TextureBudget::default();
    for part in &report.source_parts {
        let mut surface = Surface {
            first_index: part.first_index,
            index_count: part.index_count,
            visible: true,
            transparent: false,
            diffuse: None,
            normal: None,
            emissive: None,
            specular: None,
            gloss: None,
            opacity: None,
        };
        let result = (|| -> Result<(), String> {
            if part.material_slot == 255 {
                // Explicit incomplete subset table: never borrow a random
                // material from the previous CryEngine face range.
                return Ok(());
            }
            let path = material_for_part(
                &part.binding,
                part.material_hint.as_deref(),
                inventory,
                imported,
                build,
            )?
            .ok_or("No explicitly associated or same-stem MTL")?;
            let authored = read_material(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let leaf = authored
                .slots
                .get(part.material_slot as usize)
                .ok_or_else(|| {
                    format!("Original material slot {} is missing", part.material_slot)
                })?;
            if leaf.shader.eq_ignore_ascii_case("NoDraw") {
                surface.visible = false;
                return Ok(());
            }
            let ([diffuse, normal, emissive, specular, gloss, opacity], map_warnings) =
                authored_maps(
                    &authored,
                    part.material_slot,
                    inventory,
                    imported,
                    build,
                    cache,
                    &mut budget,
                )?;
            warnings.extend(
                map_warnings
                    .into_iter()
                    .map(|w| format!("{} material slot {}: {w}", part.name, part.material_slot)),
            );
            surface.diffuse = diffuse;
            surface.normal = normal;
            surface.emissive = emissive;
            surface.specular = specular;
            surface.gloss = gloss;
            surface.opacity = opacity;
            surface.transparent = needs_alpha_blending(leaf)
                && (is_translucent_shader(leaf) || surface.opacity.is_some());
            Ok(())
        })();
        if let Err(error) = result {
            warnings.push(format!("{} material: {}", part.name, error));
        }
        surfaces.push(surface);
    }
    warn_gpu_material_limit(&surfaces, &mut warnings);
    (surfaces, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_dds_counts_once_per_gpu_role_not_per_cdf_attachment() {
        let image = Arc::new(image::RgbaImage::new(4, 4));
        let surface = Surface {
            first_index: 0,
            index_count: 3,
            visible: true,
            transparent: false,
            diffuse: Some(Arc::clone(&image)),
            normal: None,
            emissive: None,
            specular: None,
            gloss: None,
            opacity: None,
        };
        let mut same = surface.clone();
        same.first_index = 3;
        assert_eq!(unique_gpu_material_bytes(&[surface, same]), 64);
    }

    #[test]
    fn historical_character_part_uses_only_its_verified_owner_material() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("characters/monsters/Rocky");
        std::fs::create_dir_all(&parent).unwrap();
        let mut old = vec![0u8; 20];
        old[..8].copy_from_slice(b"CryTek\0\0");
        old[8..12].copy_from_slice(&0xffff0000u32.to_le_bytes());
        old[12..16].copy_from_slice(&0x745u32.to_le_bytes());
        std::fs::write(parent.join("rocky3_body.chr"), &old).unwrap();
        std::fs::write(parent.join("rocky.mtl"), b"<Material/>").unwrap();
        std::fs::write(parent.join("unrelated.mtl"), b"<Material/>").unwrap();
        let inv = evolve_core::scan_directory(dir.path()).unwrap();
        let found = material_for_part(
            "objects/characters/monsters/rocky/rocky3_body.chr",
            None,
            &inv,
            None,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            found.file_name().and_then(|n| n.to_str()),
            Some("rocky.mtl")
        );
        assert!(
            material_for_part(
                "objects/characters/monsters/unrelated/rocky3_body.chr",
                None,
                &inv,
                None,
                None
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn multi_slot_mesh_without_original_subsets_must_not_use_slot_zero() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("legacy.skinm");
        let material = dir.path().join("legacy.mtl");
        // Parser must fail rather than manufacture a material assignment
        // from an unrelated mesh name or from a neighboring MTL slot.
        std::fs::write(&source, b"not a native mesh").unwrap();
        std::fs::write(&material, br#"<Material><SubMaterials><Material Name="a"/><Material Name="b"/></SubMaterials></Material>"#).unwrap();
        let inventory = evolve_core::scan_directory(dir.path()).unwrap();
        assert!(
            load_model(
                &source,
                &material,
                &inventory,
                None,
                None,
                &SharedCache::default()
            )
            .is_err()
        );
    }

    fn compiled_material_fixture(with_subsets: bool) -> Vec<u8> {
        let mut data = vec![0u8; 16 + (if with_subsets { 4 } else { 3 }) * 16];
        data[..8].copy_from_slice(b"CrChF\x07\0\0");
        data[8..12].copy_from_slice(&(if with_subsets { 4u32 } else { 3u32 }).to_le_bytes());
        data[12..16].copy_from_slice(&16u32.to_le_bytes());
        let vertices = [[0f32, 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]];
        let uvs = [[0f32, 0.], [1., 0.], [0., 1.], [1., 1.]];
        let coords = vertices
            .iter()
            .flat_map(|p| p.iter().flat_map(|c| c.to_le_bytes()))
            .collect::<Vec<_>>();
        let texcoords = uvs
            .iter()
            .flat_map(|p| p.iter().flat_map(|c| c.to_le_bytes()))
            .collect::<Vec<_>>();
        let indices = [0u16, 1, 2, 1, 3, 2]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        for (i, (kind, count, stride, bytes)) in [
            (0u32, 4u32, 12u32, coords),
            (5, 6, 2, indices),
            (2, 4, 8, texcoords),
        ]
        .into_iter()
        .enumerate()
        {
            let offset = data.len();
            let mut chunk = vec![0u8; 24];
            chunk[4..8].copy_from_slice(&kind.to_le_bytes());
            chunk[8..12].copy_from_slice(&count.to_le_bytes());
            chunk[12..16].copy_from_slice(&stride.to_le_bytes());
            chunk.extend_from_slice(&bytes);
            let table = 16 + 16 * i;
            data[table..table + 4].copy_from_slice(&0x08001016u32.to_le_bytes());
            data[table + 4..table + 8].copy_from_slice(&(i as u32).to_le_bytes());
            data[table + 8..table + 12].copy_from_slice(&(chunk.len() as u32).to_le_bytes());
            data[table + 12..table + 16].copy_from_slice(&(offset as u32).to_le_bytes());
            data.extend_from_slice(&chunk);
        }
        if with_subsets {
            let offset = data.len();
            let mut subsets = vec![0u8; 16 + 36 * 2];
            subsets[4..8].copy_from_slice(&2u32.to_le_bytes());
            subsets[16..20].copy_from_slice(&0u32.to_le_bytes());
            subsets[20..24].copy_from_slice(&3u32.to_le_bytes());
            subsets[32..36].copy_from_slice(&0u32.to_le_bytes());
            subsets[52..56].copy_from_slice(&3u32.to_le_bytes());
            subsets[56..60].copy_from_slice(&3u32.to_le_bytes());
            subsets[68..72].copy_from_slice(&1u32.to_le_bytes());
            data[64..68].copy_from_slice(&0x08001017u32.to_le_bytes());
            data[68..72].copy_from_slice(&3u32.to_le_bytes());
            data[72..76].copy_from_slice(&(subsets.len() as u32).to_le_bytes());
            data[76..80].copy_from_slice(&(offset as u32).to_le_bytes());
            data.extend_from_slice(&subsets);
        }
        data
    }

    #[test]
    fn original_material_ids_drive_nodraw_and_visible_gpu_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("hunter.skinm");
        let material = dir.path().join("hunter.mtl");
        std::fs::write(&source, compiled_material_fixture(true)).unwrap();
        std::fs::write(
            &material,
            br#"<Material Name="hunter"><SubMaterials>
                <Material Name="hidden" Shader="NoDraw"/>
                <Material Name="body"/>
            </SubMaterials></Material>"#,
        )
        .unwrap();
        let inventory = evolve_core::scan_directory(dir.path()).unwrap();
        let (surfaces, warnings) = load_model(
            &source,
            &material,
            &inventory,
            None,
            None,
            &SharedCache::default(),
        )
        .unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(surfaces.len(), 2);
        assert_eq!((surfaces[0].first_index, surfaces[0].index_count), (0, 3));
        assert_eq!((surfaces[1].first_index, surfaces[1].index_count), (3, 3));
        assert!(!surfaces[0].visible);
        assert!(surfaces[1].visible);
        std::fs::write(&source, compiled_material_fixture(false)).unwrap();
        assert!(
            load_model(
                &source,
                &material,
                &inventory,
                None,
                None,
                &SharedCache::default(),
            )
            .unwrap_err()
            .contains("no original mesh-subset table")
        );
    }

    #[test]
    fn all_supported_authored_channels_have_distinct_map_indices() {
        assert_eq!(role_channel("diffuse"), Some(0));
        assert_eq!(role_channel("normals"), Some(1));
        assert_eq!(role_channel("emittance"), Some(2));
        assert_eq!(role_channel("specular"), Some(3));
        assert_eq!(role_channel("gloss"), Some(4));
        assert_eq!(role_channel("opacity"), Some(5));
        assert_eq!(role_channel("detail"), None);
    }

    #[test]
    fn missing_opacity_map_does_not_hide_verified_diffuse() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("textures");
        std::fs::create_dir(&source).unwrap();
        let diffuse_file = source.join("color.dds");
        let mut diffuse = vec![0u8; 128 + 8];
        diffuse[0..4].copy_from_slice(b"DDS ");
        diffuse[4..8].copy_from_slice(&124u32.to_le_bytes());
        diffuse[8..12].copy_from_slice(&0x1007u32.to_le_bytes());
        diffuse[12..16].copy_from_slice(&4u32.to_le_bytes());
        diffuse[16..20].copy_from_slice(&4u32.to_le_bytes());
        diffuse[76..80].copy_from_slice(&32u32.to_le_bytes());
        diffuse[80..84].copy_from_slice(&0x4u32.to_le_bytes());
        diffuse[84..88].copy_from_slice(b"DXT1");
        std::fs::write(&diffuse_file, diffuse).unwrap();
        let document = evolve_core::parse_material(
            br#"<Material Name="armor">
            <Textures>
                <Texture Map="Diffuse" File="textures/color.dds"/>
                <Texture Map="Opacity" File="textures/missing.dds"/>
            </Textures></Material>"#,
        )
        .unwrap();
        let inventory = evolve_core::scan_directory(dir.path()).unwrap();
        let mut budget = TextureBudget::default();
        let (maps, warnings) = authored_maps(
            &document,
            0,
            &inventory,
            None,
            None,
            &SharedCache::default(),
            &mut budget,
        )
        .unwrap();
        assert!(
            maps[0].is_some(),
            "diffuse must survive a missing opacity map"
        );
        assert!(maps[5].is_none());
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("Missing exact cooked DDS"))
        );
    }

    #[test]
    fn authored_opacity_is_a_separate_map_role() {
        let document = evolve_core::parse_material(
            br#"<Material Name="sample" Shader="Illum"><Textures>
                <Texture Map="Diffuse" File="objects/diffuse.dds"/>
                <Texture Map="Opacity" File="objects/opacity.dds"/>
            </Textures></Material>"#,
        )
        .unwrap();
        let slot = &document.slots[0];
        assert!(needs_alpha_blending(slot));
        assert!(
            slot.maps
                .iter()
                .any(|reference| reference.role == "opacity")
        );
        assert!(!is_translucent_shader(slot));
    }

    #[test]
    fn ordinary_character_diffuse_alpha_does_not_make_armor_translucent() {
        let ordinary = evolve_core::MaterialSlot {
            name: "armor".into(),
            shader: "Illum".into(),
            maps: vec![evolve_core::TextureReference {
                role: "diffuse".into(),
                path: "characters/armor.dds".into(),
            }],
        };
        assert!(!needs_alpha_blending(&ordinary));
        let mut transparent = ordinary;
        transparent.maps.push(evolve_core::TextureReference {
            role: "opacity".into(),
            path: "characters/glass_mask.dds".into(),
        });
        assert!(needs_alpha_blending(&transparent));
    }

    #[test]
    fn cdf_explicit_material_is_not_replaced_with_neighboring_mtl() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("characters")).unwrap();
        std::fs::write(root.path().join("characters/weapon.mtl"), b"<Material/>").unwrap();
        let inventory = evolve_core::scan_directory(root.path()).unwrap();
        let selected = material_for_part(
            "characters/weapon.skin",
            Some("characters/missing.mtl"),
            &inventory,
            None,
            None,
        );
        assert!(selected.unwrap_err().contains("not found"));
    }
}
