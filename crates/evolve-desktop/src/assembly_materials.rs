//! Resolve each CDF attachment's original CryEngine material and cooked DDS
//! independently. All reads are source/path-checked, background-only, and
//! shared through the bounded decoded preview cache.
use crate::asset_cache::{SharedCache, decoded_dds};
use crate::auto_resources;
use crate::textured::Surface;
use evolve_core::{AssemblyReport, Inventory, MaterialDocument, material_texture_candidates, read_material};
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
        Some((stem,ext)) if ext.eq_ignore_ascii_case("skin") => format!("{stem}.skinm"),
        Some((stem,ext)) if ext.eq_ignore_ascii_case("chr") => format!("{stem}.chrm"),
        _ => binding.to_owned(),
    };
    let path = auto_resources::exact(inventory, &rendering, imported, build)?
        .or(auto_resources::exact(inventory, binding, imported, build)?);
    let Some(path) = path else { return Ok(Vec::new()) };
    let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() > 16*1024*1024 {
        return Ok(Vec::new());
    }
    let data = std::fs::read(&path).map_err(|e| e.to_string())?;
    if data.len() < 32 || data.get(..5) != Some(b"CrChF") {
        return Ok(Vec::new());
    }
    let u32_at = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(data.get(at..at+4)?.try_into().ok()?))
    };
    let Some(count) = u32_at(8).map(|v|v as usize) else { return Ok(Vec::new()) };
    let Some(start) = u32_at(12).map(|v|v as usize) else { return Ok(Vec::new()) };
    if count == 0 || count > 10000 || start < 16
        || start.checked_add(count*16).is_none_or(|end|end>data.len())
    {
        return Ok(Vec::new());
    }
    let mut hints = Vec::new();
    for i in 0..count {
        let at = start+i*16;
        if u32_at(at) != Some(0x08021014) { continue; }
        let (Some(len),Some(offset)) = (u32_at(at+8),u32_at(at+12)) else { continue };
        let (len,offset)=(len as usize,offset as usize);
        if len < 132 || offset < start+count*16
            || offset.checked_add(len).is_none_or(|end|end>data.len()) { continue }
        let Some(field) = data.get(offset..offset+128) else { continue };
        let Some(zero) = field.iter().position(|b| *b == 0) else { continue };
        let Ok(name) = std::str::from_utf8(&field[..zero]) else { continue };
        let hint = name.trim().replace('\\',"/");
        if hint.is_empty() { continue }
        let hint = if hint.to_ascii_lowercase().ends_with(".mtl") {
            hint
        } else { format!("{hint}.mtl") };
        if !hints.iter().any(|value: &String| value.eq_ignore_ascii_case(&hint)) {
            hints.push(hint);
        }
    }
    Ok(hints)
}

fn material_for_part(
    binding: &str,
    explicit: Option<&str>,
    inventory: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    if let Some(explicit) = explicit {
        // An explicit override is authoritative. Do not substitute an
        // unrelated same-named material if its source cannot be found.
        return auto_resources::exact(inventory, explicit, imported, build)?
            .ok_or_else(|| format!("Source CDF material not found: {explicit}"))
            .map(Some);
    }
    // Compiled MtlName data is authoritative before a fallback same-stem
    // material; multiple contradictory authored hints remain ambiguous.
    let parent = binding.rsplit_once('/').map_or("", |(folder,_)| folder);
    let embedded = embedded_material_hints(binding, inventory, imported, build)?;
    let mut unique = Vec::new();
    for hint in embedded {
        let mut paths = vec![hint.clone()];
        if !hint.starts_with("characters/") && !hint.starts_with("objects/")
            && !hint.starts_with("materials/") && !parent.is_empty()
        {
            paths.insert(0, format!("{parent}/{hint}"));
        }
        for path in paths {
            if let Some(found) = auto_resources::exact(inventory, &path, imported, build)?
                && !unique.contains(&found) {
                    unique.push(found);
            }
        }
    }
    if unique.len() > 1 {
        return Err(format!("Conflicting compiled material references: {binding}"));
    }
    if let Some(found) = unique.pop() {
        return Ok(Some(found));
    }
    // Only a same-stem material is considered, never a random DDS or a
    // different character's material from another shared archive.
    let stem = binding.rsplit('/').next().unwrap_or(binding)
        .rsplit_once('.').map_or(binding, |(name,_)| name);
    let mut bases = vec![stem.to_owned()];
    let lower = stem.to_ascii_lowercase();
    if let Some(at) = lower.rfind("_lod") {
        if lower[at+4..].bytes().all(|c| c.is_ascii_digit()) {
            bases.push(stem[..at].to_owned());
        }
    }
    for base in bases {
        let path = if parent.is_empty() { format!("{base}.mtl") }
            else { format!("{parent}/{base}.mtl") };
        if let Some(found) = auto_resources::exact(inventory, &path, imported, build)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn authored_maps(
    document: &MaterialDocument,
    material_slot: u32,
    inventory: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
    cache: &SharedCache,
    bytes_loaded: &mut usize,
    decoded: &mut HashSet<PathBuf>,
) -> Result<[Option<Arc<image::RgbaImage>>; 5], String> {
    let slot = document.slots.get(material_slot as usize).ok_or_else(||
        format!("Source material slot {} exceeds {} authored leaves",
            material_slot, document.slots.len()))?;
    let mut result: [Option<Arc<image::RgbaImage>>; 5] = std::array::from_fn(|_| None);
    for map in &slot.maps {
        let index = match map.role.as_str() {
            "diffuse" => 0,
            "normals" | "normal" | "bumpmap" => 1,
            "emittance" | "emissive" | "emission" | "glow" => 2,
            "specular" | "specularcolor" => 3,
            "gloss" | "glossiness" | "smoothness" => 4,
            _ => continue,
        };
        if result[index].is_some() {
            return Err(format!("Multiple authored {} maps; no arbitrary layer selection", map.role));
        }
        let mut found = None;
        for candidate in material_texture_candidates(&map.path) {
            if let Some(path) = auto_resources::exact(inventory, &candidate, imported, build)? {
                found = Some(path);
                break;
            }
        }
        let Some(path) = found else {
            return Err(format!("Missing exact cooked DDS for {}",map.path));
        };
        let canonical = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
        let (_info, image) = decoded_dds(cache, &canonical)?;
        let required = image.as_raw().len();
        if !decoded.contains(&canonical) {
            *bytes_loaded = bytes_loaded.checked_add(required)
                .ok_or("Decoded CDF texture budget overflow")?;
            if *bytes_loaded > MAX_DECODED_ASSEMBLY_BYTES {
                return Err("CDF texture budget exceeded (768 MiB)".into());
            }
            decoded.insert(canonical);
        }
        result[index] = Some(image);
    }
    Ok(result)
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
    let mut bytes_loaded = 0usize;
    let mut decoded = HashSet::new();
    for part in &report.source_parts {
        let mut surface = Surface {
            first_index: part.first_index,
            index_count: part.index_count,
            diffuse: None,
            normal: None,
            emissive: None,
            specular: None,
            gloss: None,
        };
        let result = (|| -> Result<(),String> {
            if part.material_slot == 255 {
                // Explicit incomplete subset table: never borrow a random
                // material from the previous CryEngine face range.
                return Ok(());
            }
            let path = material_for_part(
                &part.binding, part.material_hint.as_deref(), inventory, imported, build
            )?.ok_or("No explicitly associated or same-stem MTL")?;
            let authored = read_material(&path).map_err(|e| format!("{}: {e}",path.display()))?;
            let [diffuse, normal, emissive, specular, gloss] =
                authored_maps(&authored, part.material_slot, inventory, imported, build, cache,
                    &mut bytes_loaded, &mut decoded)?;
            surface.diffuse = diffuse;
            surface.normal = normal;
            surface.emissive = emissive;
            surface.specular = specular;
            surface.gloss = gloss;
            Ok(())
        })();
        if let Err(error) = result {
            warnings.push(format!("{} material: {}", part.name, error));
        }
        surfaces.push(surface);
    }
    (surfaces,warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cdf_explicit_material_is_not_replaced_with_neighboring_mtl() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("characters")).unwrap();
        std::fs::write(root.path().join("characters/weapon.mtl"),b"<Material/>").unwrap();
        let inventory = evolve_core::scan_directory(root.path()).unwrap();
        let selected = material_for_part(
            "characters/weapon.skin",Some("characters/missing.mtl"),
            &inventory,None,None);
        assert!(selected.unwrap_err().contains("not found"));
    }
}
