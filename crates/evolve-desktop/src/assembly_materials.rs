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
    // Only a same-stem material is considered, never a random DDS or a
    // different character's material from another shared archive.
    let parent = binding.rsplit_once('/').map_or("", |(folder,_)| folder);
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
