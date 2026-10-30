//! Resolve each CDF attachment's original CryEngine material and cooked DDS
//! independently. All reads are source/path-checked, background-only, and
//! shared through the bounded decoded preview cache.
use crate::asset_cache::{SharedCache, decoded_dds_indexed};
use crate::auto_resources;
use crate::textured::{MAX_GPU_PREVIEW_TEXTURE_BYTES, Surface};
use evolve_core::{
    AssemblyReport, Inventory, MaterialDocument, read_material, scoped_texture_candidates,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_DECODED_ASSEMBLY_BYTES: usize = 768 * 1024 * 1024;

/// Exact authored DDS color channels are sRGB in the wgpu sampler. Calculate
/// armor differences in linear light, never clipping them at 80/255.
fn srgb_linear(byte: u8) -> f32 {
    let x = f32::from(byte) / 255.0;
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}
fn linear_srgb(value: f32) -> u8 {
    let x = value.clamp(0.0, 1.0);
    let encoded = if x <= 0.0031308 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round().clamp(0.0, 255.0) as u8
}

/// A legacy positive GlowAmount permits diffuse-alpha emission only with
/// actual alpha-mask evidence. Alpha alone may be gloss or opacity and is not
/// a license to invent emission for unrelated materials.
fn legacy_alpha_emission(
    slot: &evolve_core::MaterialSlot,
    diffuse: &image::RgbaImage,
) -> Option<image::RgbaImage> {
    if !matches!(
        slot.shader.to_ascii_lowercase().as_str(),
        "illum" | "cloth" | "humanskin"
    ) || !slot.alpha_glow
        || !slot.glow_amount.is_some_and(|amount| amount > 0.0)
        || !diffuse.pixels().any(|pixel| pixel[3] < 255)
    {
        return None;
    }
    let mut result = diffuse.clone();
    for pixel in result.pixels_mut() {
        for channel in 0..3 {
            pixel[channel] = linear_srgb(srgb_linear(pixel[channel]) * f32::from(pixel[3]) / 255.0);
        }
        pixel[3] = 255;
    }
    Some(result)
}

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
    // 2012 source folders often have a build suffix (Goliath_2,
    // Scorpid_2), while authored render parts use a character/skin suffix
    // (Goliath3_Rig.chr). Compare the original character-family prefix,
    // not the build folder's entire basename.
    let folder_family = folder.split('_').next().unwrap_or(folder);
    if folder_family.len() < 4
        || !stem
            .to_ascii_lowercase()
            .starts_with(&folder_family.to_ascii_lowercase())
    {
        return Ok(None);
    }
    let suffix = &stem[folder_family.len()..];
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
    // First consider the exact original variant stem in the same folder.
    // Goliath3_Rig / Goliath3_Default_Head may have a common Goliath3.mtl,
    // not a Goliath_2_Rig.mtl. Do not glob for similarly named characters.
    if let Some((variant, _)) = stem.split_once('_')
        && variant.len() >= folder_family.len()
    {
        let candidate = format!("{parent}/{variant}.mtl");
        if let Some(file) = auto_resources::exact(inventory, &candidate, imported, build)? {
            return Ok(Some(file));
        }
    }
    for candidate in [
        format!("{parent}/{folder}.mtl"),
        format!("{parent}/{folder_family}.mtl"),
    ] {
        if let Some(file) = auto_resources::exact(inventory, &candidate, imported, build)? {
            return Ok(Some(file));
        }
    }
    Ok(None)
}

pub(crate) fn material_for_part(
    binding: &str,
    explicit: Option<&str>,
    inventory: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    if let Some(explicit) = explicit {
        // Historical CDF <Model Material="..."> and CA_SKIN materials often
        // reference an exact MTL path *without* the .mtl extension.
        // Normalizing the extension is not filename-guessing.
        let explicit = explicit.trim().replace('\\', "/");
        let lower = explicit.to_ascii_lowercase();
        let virtual_mtl = if lower.ends_with(".mtl") {
            explicit
        } else if Path::new(&explicit).extension().is_none() {
            format!("{explicit}.mtl")
        } else {
            return Err(format!(
                "CDF material has unsupported extension: {explicit}"
            ));
        };
        let parent = binding.rsplit_once('/').map_or("", |(folder, _)| folder);
        let normalized = virtual_mtl.to_ascii_lowercase();
        let root_path = normalized.starts_with("characters/")
            || normalized.starts_with("objects/")
            || normalized.starts_with("materials/");
        if !root_path && !parent.is_empty() {
            let scoped = format!("{parent}/{virtual_mtl}");
            if let Some(found) = auto_resources::exact(inventory, &scoped, imported, build)? {
                return Ok(Some(found));
            }
        }
        return auto_resources::exact(inventory, &virtual_mtl, imported, build)?
            .ok_or_else(|| format!("Source CDF material not found: {virtual_mtl}"))
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
        "emittance" | "emissive" | "emission" | "glow" | "emissivecolor" | "emissionmap"
        | "glowmap" | "selfillum" | "selfillumination" => Some(2),
        "specular" | "specularcolor" => Some(3),
        "gloss" | "glossiness" | "smoothness" => Some(4),
        "opacity" => Some(5),
        _ => None,
    }
}

/// An explicitly authored Illum Decal with positive GlowAmount is a
/// separate legacy glow source, not a second diffuse-alpha mask. Kala's
/// Banshee Gun Glow submaterial deliberately lacks the ALPHAGLOW flag.
fn legacy_decal_role(slot: &evolve_core::MaterialSlot, role: &str) -> Option<usize> {
    if role == "decal"
        && slot.shader.eq_ignore_ascii_case("Illum")
        && slot.glow_amount.is_some_and(|amount| amount > 0.0)
        && !slot.alpha_glow
    {
        Some(2)
    } else {
        role_channel(role)
    }
}

/// The authored Decal transform affects only emission; it does not move UV0
/// for diffuse, normals, opacity, or original character geometry.
fn source_decal_motion(slot: &evolve_core::MaterialSlot) -> Option<evolve_core::DecalTexMod> {
    let source = slot.decal_tex_mod?;
    let emitted = slot
        .maps
        .iter()
        .any(|map| map.role == "decal" && legacy_decal_role(slot, &map.role) == Some(2));
    let other_emission = slot
        .maps
        .iter()
        .any(|map| map.role != "decal" && role_channel(&map.role) == Some(2));
    if emitted && !other_emission {
        Some(source)
    } else {
        None
    }
}

/// Kala's animated chroma-fan atlas contains different source RGB hues.
/// Its dedicated Banshee Gun Glow leaf instead authors a blue Diffuse color.
/// Use the atlas's linear brightness as the independent moving emission mask:
/// RGB-by-RGB multiplication retained the atlas's red frames as red streaks.
/// Restrict this source-specific mask interpretation to the verified material;
/// other legacy Decals keep their authored per-channel texture colors.
fn kala_banshee_brightness_mask(slot: &evolve_core::MaterialSlot) -> bool {
    slot.name.eq_ignore_ascii_case("bansheegun glow")
        && slot.diffuse_tint.is_some()
        && slot.maps.iter().any(|map| {
            map.role == "decal"
                && map
                    .path
                    .rsplit('/')
                    .next()
                    .is_some_and(|file| file.eq_ignore_ascii_case("chroma_fan320tiled_01b.tif"))
        })
}

fn kala_tubing_leaf(
    material_source: &Path,
    binding: &str,
    slot: &evolve_core::MaterialSlot,
) -> bool {
    let path = material_source
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let source = binding.replace('\\', "/").to_ascii_lowercase();
    let authored_decal = slot.maps.iter().any(|map| {
        map.role == "decal"
            && map
                .path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("chroma_fan320tiled_01b.tif"))
    });
    authored_decal
        && ((path.ends_with("/merc_tech/tech_3p.mtl")
            && source.ends_with("/merc_tech/tech_3p.skin")
            && slot.name.eq_ignore_ascii_case("Glow"))
            || (path.ends_with("/merc_tech/weapons/bansheegun/techbansheegun_3p.mtl")
                && source.ends_with("/merc_tech/techbansheegun_3p.skin")
                && (slot.name.eq_ignore_ascii_case("bansheegun glow")
                    || slot.name.eq_ignore_ascii_case("armorreducer glow"))))
}

fn tint_legacy_decal(
    slot: &evolve_core::MaterialSlot,
    decal: &image::RgbaImage,
) -> image::RgbaImage {
    tint_legacy_decal_for_state(slot, decal, false)
}

fn tint_legacy_decal_for_state(
    slot: &evolve_core::MaterialSlot,
    decal: &image::RgbaImage,
    kala_tubing: bool,
) -> image::RgbaImage {
    let tint = slot.diffuse_tint.unwrap_or([1.0, 1.0, 1.0]);
    let use_brightness_mask = kala_tubing || kala_banshee_brightness_mask(slot);
    let mut result = decal.clone();
    for pixel in result.pixels_mut() {
        let alpha = f32::from(pixel[3]) / 255.0;
        let channels = [
            srgb_linear(pixel[0]),
            srgb_linear(pixel[1]),
            srgb_linear(pixel[2]),
        ];
        let brightness = channels[0].max(channels[1]).max(channels[2]);
        for channel in 0..3 {
            let source = if use_brightness_mask {
                brightness
            } else {
                channels[channel]
            };
            pixel[channel] = linear_srgb(source * tint[channel].clamp(0.0, 1.0) * alpha);
        }
        pixel[3] = 255;
    }
    result
}

/// Keep material-relative paths in the original indexed namespace; this is
/// not a basename scan over unrelated actors or builds. Imported PAK sources
/// live at <library>/<archive>/files/<game-virtual-path>.
fn material_virtual_owner(
    material: &Path,
    fallback_binding: &str,
    inventory: &Inventory,
    imported: Option<&Path>,
) -> String {
    if let Ok(real) = std::fs::canonicalize(material) {
        if let Ok(root) = std::fs::canonicalize(&inventory.root)
            && let Ok(relative) = real.strip_prefix(root)
        {
            return relative.to_string_lossy().replace('\\', "/");
        }
        if let Some(imported) = imported
            && let Ok(root) = std::fs::canonicalize(imported)
            && let Ok(relative) = real.strip_prefix(root)
        {
            let mut components = relative.components();
            if components.next().is_some()
                && components
                    .next()
                    .is_some_and(|component| component.as_os_str() == "files")
            {
                let virtual_path = components.as_path();
                if !virtual_path.as_os_str().is_empty() {
                    return virtual_path.to_string_lossy().replace('\\', "/");
                }
            }
        }
    }
    fallback_binding.to_owned()
}

fn authored_texture(
    reference: &str,
    material: &Path,
    binding: &str,
    inventory: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<(PathBuf, String), String> {
    let owner = material_virtual_owner(material, binding, inventory, imported);
    let candidates = scoped_texture_candidates(reference, &owner)?;
    for candidate in &candidates {
        if let Some(path) = auto_resources::exact(inventory, candidate, imported, build)? {
            // The core imported resolver may normalize a .dds request to a
            // preferred .dds.0. Keep the exact cooked fragment identity for
            // subsequent cross-archive high-mip reconstruction.
            let relative = if candidate.to_ascii_lowercase().ends_with(".dds")
                && path.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .to_ascii_lowercase()
                        .ends_with(".dds.0")
                }) {
                format!("{candidate}.0")
            } else {
                candidate.clone()
            };
            return Ok((path, relative));
        }
    }
    Err(format!(
        "Missing exact cooked DDS for {reference} in material {owner}; checked {} source-qualified paths{} (build: {}). Confirm the original texture PAK for this build was imported. First candidates: {}",
        candidates.len(),
        if imported.is_some() {
            " in the selected archive and imported library"
        } else {
            " in the selected archive"
        },
        build.unwrap_or("unselected"),
        candidates
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
    ))
}

/// Goliath's authored Illum BLENDLAYER materials bind a damage-blend color
/// mask in an XML channel named "Opacity". It controls the alternate damage
/// layer, not the mesh's coverage. Never discard solid body pixels using it.
/// The exact authoring convention is also present in Goliath 1 and Goliath 2.
fn is_damage_blend_mask(
    slot: &evolve_core::MaterialSlot,
    map: &evolve_core::TextureReference,
) -> bool {
    slot.shader.eq_ignore_ascii_case("Illum")
        && map.role == "opacity"
        && map.path.to_ascii_lowercase().contains("damage_blend_diff")
}

/// CryEngine's generic Opacity channel can hold non-cutout shader data on
/// solid monster and equipment meshes. A map by itself does not establish
/// alpha-test semantics. Real cutout/alpha-labelled sources are preserved.
fn unverified_solid_body_opacity(slot: &evolve_core::MaterialSlot) -> bool {
    matches!(
        slot.shader.to_ascii_lowercase().as_str(),
        "illum" | "cloth" | "humanskin"
    ) && slot.opacity.unwrap_or(1.0) >= 0.999
        && !slot.maps.iter().any(|map| {
            map.role == "opacity"
                && ["cutout", "alphatest", "_alpha"]
                    .iter()
                    .any(|marker| map.path.to_ascii_lowercase().contains(marker))
        })
}

/// CryEngine's Vegetation shader uses diffuse DDS alpha for leaf/grass card
/// coverage. Its separate Opacity map is thickness/backlight transmittance,
/// not the shape of the leaf. Require the original shader identity; never
/// interpret the alpha of ordinary Illum/character diffuse maps as coverage.
fn source_vegetation_cutout(slot: &evolve_core::MaterialSlot) -> bool {
    slot.shader.eq_ignore_ascii_case("Vegetation") && slot.opacity.unwrap_or(1.0) >= 0.999
}

/// Per-channel resolved source textures and independent missing-map diagnostics.
type ResolvedMaterialMaps = ([Option<Arc<image::RgbaImage>>; 6], Vec<String>);

/// Keep source/build provenance together; the lookup is unchanged for each map.
#[derive(Clone, Copy)]
struct MaterialLookup<'a> {
    inventory: &'a Inventory,
    imported: Option<&'a Path>,
    build: Option<&'a str>,
}

// The original material provenance, texture budget, and Kala-only viewer
// state are intentionally explicit: never resolve a texture by filename alone.
#[allow(clippy::too_many_arguments)]
fn authored_maps(
    document: &MaterialDocument,
    material_source: &Path,
    material_binding: &str,
    material_slot: u32,
    lookup: MaterialLookup<'_>,
    cache: &SharedCache,
    budget: &mut TextureBudget,
    kala_state: Option<evolve_core::KalaTubingState>,
) -> Result<ResolvedMaterialMaps, String> {
    let MaterialLookup {
        inventory,
        imported,
        build,
    } = lookup;
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
        if !is_damage_blend_mask(slot, map)
            && let Some(index) = legacy_decal_role(slot, &map.role)
        {
            counts[index] += 1;
        }
    }
    // A uniquely authored Emittance entry outranks alternate spellings
    // when both appear. Two different Emittance maps remain ambiguous.
    let emittance = slot
        .maps
        .iter()
        .filter(|map| map.role == "emittance")
        .collect::<Vec<_>>();
    let primary_emittance = (emittance.len() == 1).then(|| emittance[0].path.as_str());
    for map in &slot.maps {
        if is_damage_blend_mask(slot, map) {
            warnings.push(
                "Original CryEngine damage-blend mask is not transparency; enable the sourced damage diffuse in Model Options to preview the blend".into(),
            );
            continue;
        }
        let Some(index) = legacy_decal_role(slot, &map.role) else {
            continue;
        };
        if index == 5 && source_vegetation_cutout(slot) {
            // This image controls foliage transmittance, NOT the alpha-test
            // silhouette. Deferred lighting can bind it independently later.
            continue;
        }
        if index == 5 && unverified_solid_body_opacity(slot) {
            warnings.push(format!(
                "{}: ignored an unverified opacity cutout on opaque {} body material; base diffuse remains visible",
                slot.name, slot.shader
            ));
            continue;
        }
        if index == 2 && counts[index] > 1 && primary_emittance.is_some() {
            if Some(map.path.as_str()) != primary_emittance {
                continue;
            }
            warnings.push(format!(
                "{}: selected uniquely authored Emittance over other emission-role maps",
                slot.name
            ));
        } else if counts[index] > 1 {
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
            let (path, virtual_path) = authored_texture(
                &map.path,
                material_source,
                material_binding,
                inventory,
                imported,
                build,
            )?;
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
        if index == 2 && map.role == "decal" {
            let verified_kala =
                kala_state.is_some() && kala_tubing_leaf(material_source, material_binding, slot);
            // The original tech_3p.mtl body Glow stays blue in every
            // equipment state. Only the equipment's own authored glow leaf
            // can be red; never recolor body/waist tubing via weapon state.
            result[index] = Some(Arc::new(if verified_kala {
                tint_legacy_decal_for_state(slot, &image, true)
            } else {
                tint_legacy_decal(slot, &image)
            }));
            warnings.push(format!(
                "{}: legacy Illum Decal uses authored GlowAmount and Diffuse tint, without diffuse-alpha masking; the viewer animates the Decal's independent TexMod (FBX stores its original map without baked motion)",
                slot.name
            ));
        } else {
            result[index] = Some(image);
        }
    }
    if result[2].is_none()
        && !slot
            .maps
            .iter()
            .any(|map| role_channel(&map.role) == Some(2))
        && let Some(base) = result[0].as_ref()
        && let Some(mask) = legacy_alpha_emission(slot, base)
    {
        warnings.push(format!(
            "Source GlowAmount={} uses a diffuse-alpha emission mask (preview approximation)",
            slot.glow_amount.unwrap_or(0.0)
        ));
        result[2] = Some(Arc::new(mask));
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
    // An explicit opacity IMAGE on a solid CryEngine material is an alpha
    // TEST, not evidence the entire body is translucent. Blend only for
    // glass/water or for an explicit subunit scalar without a cutout map.
    // Solid masked faces must write depth so floor/background cannot show
    // through Goliath's body when the mask has intermediate gray values.
    is_translucent_shader(slot)
        || (slot.opacity.is_some_and(|value| value < 0.999)
            && !slot
                .maps
                .iter()
                .any(|map| map.role == "opacity" && !is_damage_blend_mask(slot, map)))
}

/// The authoring model, not the diffuse bitmap's fourth channel, decides
/// whether this surface transmits light. A Glass shader with Opacity≈1 uses
/// a documented, neutral preview approximation of refraction, not the
/// frequently unrelated CryEngine gloss/mask stored in diffuse alpha.
fn verified_opacity(
    slot: &evolve_core::MaterialSlot,
    mask: Option<Arc<image::RgbaImage>>,
) -> Option<Arc<image::RgbaImage>> {
    let scalar = slot.opacity.unwrap_or(1.0);
    let factor = if is_translucent_shader(slot) && scalar >= 0.98 {
        0.40
    } else {
        scalar
    };
    if let Some(source) = mask {
        if factor >= 0.999 {
            return Some(source);
        }
        let mut pixels = source.as_ref().clone();
        for pixel in pixels.pixels_mut() {
            pixel.0[0] = (f32::from(pixel.0[0]) * factor).round() as u8;
        }
        return Some(Arc::new(pixels));
    }
    if factor >= 0.999 {
        return None;
    }
    Some(Arc::new(image::RgbaImage::from_pixel(
        1,
        1,
        image::Rgba([(factor * 255.0).round() as u8, 255, 255, 255]),
    )))
}

/// Load exact source subset/material IDs for a standalone .skinm/.chrm.
/// The ordinary model viewer previously resolved an MTL's textures but never
/// built the per-subset GPU surfaces, so 11-slot models appeared untextured.
/// World CGF files can hold multiple independently NODE-referenced meshes
/// while the authored MTL has just one leaf. In that specific unambiguous
/// case every section receives the *same original* material without guessing
/// submesh slot IDs or failing the multi-stream source decoder.
pub fn original_single_leaf_surface(
    material: &Path,
    inventory: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
    cache: &SharedCache,
) -> Result<(Surface, Vec<String>), String> {
    let document = read_material(material).map_err(|e| e.to_string())?;
    if document.slots.len() != 1 {
        return Err(format!(
            "{} MTL leaves require the source per-mesh subset table",
            document.slots.len()
        ));
    }
    let leaf = &document.slots[0];
    let visible = !leaf.shader.eq_ignore_ascii_case("NoDraw");
    let mut surface = Surface {
        first_index: 0,
        index_count: 0, // filled from each *verified* mesh's index count
        visible,
        transparent: false,
        foliage_cutout: false,
        alpha_cutoff: 0.5,
        diffuse: None,
        normal: None,
        emissive: None,
        armor_emissive: None,
        emission_strength: leaf.glow_amount.unwrap_or(1.0).max(0.0),
        kala_emission_state: None,
        emissive_uv: source_decal_motion(leaf),
        specular: None,
        gloss: None,
        opacity: None,
    };
    if !visible {
        return Ok((surface, Vec::new()));
    }
    let (maps, warnings) = authored_maps(
        &document,
        material,
        &material.to_string_lossy(),
        0,
        MaterialLookup {
            inventory,
            imported,
            build,
        },
        cache,
        &mut TextureBudget::default(),
        None,
    )?;
    let [diffuse, normal, emissive, specular, gloss, opacity] = maps;
    surface.diffuse = diffuse;
    surface.normal = normal;
    surface.emissive = emissive;
    surface.specular = specular;
    surface.gloss = gloss;
    surface.foliage_cutout = source_vegetation_cutout(leaf) && surface.diffuse.is_some();
    surface.alpha_cutoff = leaf.alpha_test.unwrap_or(0.5);
    surface.opacity = if surface.foliage_cutout {
        None
    } else {
        verified_opacity(leaf, opacity)
    };
    surface.transparent =
        !surface.foliage_cutout && needs_alpha_blending(leaf) && surface.opacity.is_some();
    Ok((surface, warnings))
}

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
            foliage_cutout: false,
            alpha_cutoff: 0.5,
            diffuse: None,
            normal: None,
            emissive: None,
            armor_emissive: None,
            emission_strength: 1.0,
            kala_emission_state: None,
            emissive_uv: None,
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
                    material,
                    &source.to_string_lossy(),
                    material_slot,
                    MaterialLookup {
                        inventory,
                        imported,
                        build,
                    },
                    cache,
                    &mut budget,
                    None,
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
                        surface.emission_strength = leaf.glow_amount.unwrap_or(1.0).max(0.0);
                        surface.emissive_uv = source_decal_motion(leaf);
                        surface.specular = specular;
                        surface.gloss = gloss;
                        // Original scalar opacity, when present, is independent
                        // of packed diffuse DDS alpha. Never blend an Illum
                        // body unless there is explicit authored evidence.
                        surface.foliage_cutout =
                            source_vegetation_cutout(leaf) && surface.diffuse.is_some();
                        surface.alpha_cutoff = leaf.alpha_test.unwrap_or(0.5);
                        surface.opacity = if surface.foliage_cutout {
                            None
                        } else {
                            verified_opacity(leaf, opacity)
                        };
                        surface.transparent = !surface.foliage_cutout
                            && needs_alpha_blending(leaf)
                            && surface.opacity.is_some();
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

/// Apply the material's authored BLENDLAYER over the exact original diffuse.
/// This runs off the UI thread and only changes ephemeral preview pixels.
/// The mask is NOT an alpha test: source Illum mesh coverage remains opaque.
pub fn apply_damage_blend(
    report: &AssemblyReport,
    surfaces: &mut [Surface],
    percent: u8,
    inventory: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
    cache: &SharedCache,
) -> Result<usize, String> {
    if percent == 0 {
        return Ok(0);
    }
    if percent > 100 || report.source_parts.len() != surfaces.len() {
        return Err("Invalid model damage-blend selection".into());
    }
    let mut applied = 0usize;
    let mut used = 0usize;
    let mut previous = std::collections::HashMap::new();
    for (part, surface) in report.source_parts.iter().zip(surfaces.iter_mut()) {
        let Some(base) = surface.diffuse.as_ref() else {
            continue;
        };
        if !surface.visible || surface.transparent {
            continue;
        }
        let Some(material) = material_for_part(
            &part.binding,
            part.material_hint.as_deref(),
            inventory,
            imported,
            build,
        )?
        else {
            continue;
        };
        let doc = read_material(&material).map_err(|e| e.to_string())?;
        let Some(leaf) = doc.slots.get(part.material_slot as usize) else {
            continue;
        };
        let Some((damage_ref, mask_ref)) = evolve_core::authored_damage_layer(leaf) else {
            continue;
        };
        let resolve = |reference: &str| -> Result<Arc<image::RgbaImage>, String> {
            let (source, virtual_path) = authored_texture(
                reference,
                &material,
                &part.binding,
                inventory,
                imported,
                build,
            )?;
            let canonical = std::fs::canonicalize(source).map_err(|e| e.to_string())?;
            decoded_dds_indexed(cache, &canonical, &virtual_path, inventory, imported, build)
                .map(|(_, image)| image)
        };
        let alternate = resolve(damage_ref)?;
        let mask = resolve(mask_ref)?;
        let key = (
            Arc::as_ptr(base) as usize,
            Arc::as_ptr(&alternate) as usize,
            Arc::as_ptr(&mask) as usize,
        );
        let blended = if let Some(image) = previous.get(&key) {
            Arc::clone(image)
        } else {
            let pixels = evolve_core::blend_damage_rgba(base, &alternate, &mask, percent)?;
            used = used
                .checked_add(pixels.as_raw().len())
                .ok_or("Damage layer memory overflow")?;
            if used > MAX_GPU_PREVIEW_TEXTURE_BYTES {
                return Err("Damage layer exceeds 256 MiB GPU preview budget".into());
            }
            let image = Arc::new(pixels);
            previous.insert(key, Arc::clone(&image));
            image
        };
        surface.diffuse = Some(blended);
        applied += 1;
    }
    if applied == 0 {
        return Err("No verified authored damage Custom + Opacity blend pair exists in the selected monster CDF materials".into());
    }
    Ok(applied)
}

/// Blend cached 0% and 100% authored damage previews without opening the
/// CDF, scanning any archives or decoding any DDS again. Geometry, per-subset
/// material bindings and all unrelated GPU maps are shared by Arc.
pub fn blend_prepared_damage(
    base: &[Surface],
    full: &[Surface],
    percent: u8,
) -> Result<Vec<Surface>, String> {
    if percent > 100 || base.len() != full.len() {
        return Err("Mismatched cached damage preview".into());
    }
    if percent == 0 {
        return Ok(base.to_vec());
    }
    if percent == 100 {
        return Ok(full.to_vec());
    }
    let mut out = base.to_vec();
    let mut derived = HashMap::<(usize, usize), Arc<image::RgbaImage>>::new();
    let mut total = 0usize;
    for ((original, damaged), target) in base.iter().zip(full).zip(out.iter_mut()) {
        if original.first_index != damaged.first_index
            || original.index_count != damaged.index_count
            || original.visible != damaged.visible
        {
            return Err("Cached damage material ranges changed".into());
        }
        let (Some(before), Some(after)) = (&original.diffuse, &damaged.diffuse) else {
            continue;
        };
        if Arc::ptr_eq(before, after) {
            continue;
        }
        if before.dimensions() != after.dimensions() {
            return Err("Cached damage dimensions changed".into());
        }
        let key = (Arc::as_ptr(before) as usize, Arc::as_ptr(after) as usize);
        let image = if let Some(reused) = derived.get(&key) {
            Arc::clone(reused)
        } else {
            let len = before.as_raw().len();
            total = total
                .checked_add(len)
                .ok_or("Damage blend memory overflow")?;
            if total > MAX_GPU_PREVIEW_TEXTURE_BYTES {
                return Err("Damage preview exceeds its GPU image budget".into());
            }
            let amount = u32::from(percent);
            let pixels = before
                .as_raw()
                .as_chunks::<4>()
                .0
                .iter()
                .zip(after.as_raw().as_chunks::<4>().0.iter())
                .flat_map(|(a, b)| {
                    [
                        ((u32::from(a[0]) * (100 - amount) + u32::from(b[0]) * amount + 50) / 100)
                            as u8,
                        ((u32::from(a[1]) * (100 - amount) + u32::from(b[1]) * amount + 50) / 100)
                            as u8,
                        ((u32::from(a[2]) * (100 - amount) + u32::from(b[2]) * amount + 50) / 100)
                            as u8,
                        255u8,
                    ]
                })
                .collect::<Vec<u8>>();
            let blended = Arc::new(
                image::RgbaImage::from_raw(before.width(), before.height(), pixels)
                    .ok_or("Cached damage pixel count changed")?,
            );
            derived.insert(key, Arc::clone(&blended));
            blended
        };
        target.diffuse = Some(image);
    }
    Ok(out)
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
            &surface.armor_emissive,
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

/// Per-material-range progress reports completed source work, not elapsed time.
/// The source CDF is read only; this never synthesizes texture assignments.
pub fn load_with_progress(
    report: &AssemblyReport,
    inventory: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
    cache: &SharedCache,
    progress: Option<&std::sync::atomic::AtomicUsize>,
    kala_state: evolve_core::KalaTubingState,
) -> (Vec<Surface>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut surfaces = Vec::with_capacity(report.source_parts.len());
    let mut budget = TextureBudget::default();
    // CDFs often repeat a material for multiple subsets. Resolve the
    // authored source, parse XML and find the same decoded DDS once per
    // binding/slot, not once per triangle range.
    let mut resolved_materials =
        HashMap::<(String, Option<String>), Result<Option<PathBuf>, String>>::new();
    let mut parsed_materials =
        HashMap::<PathBuf, Result<evolve_core::MaterialDocument, String>>::new();
    let mut decoded_slots =
        HashMap::<(PathBuf, String, u32), Result<ResolvedMaterialMaps, String>>::new();
    for (index, part) in report.source_parts.iter().enumerate() {
        let mut surface = Surface {
            first_index: part.first_index,
            index_count: part.index_count,
            visible: true,
            transparent: false,
            foliage_cutout: false,
            alpha_cutoff: 0.5,
            diffuse: None,
            normal: None,
            emissive: None,
            armor_emissive: None,
            emission_strength: 1.0,
            kala_emission_state: None,
            emissive_uv: None,
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
            let binding_key = (part.binding.clone(), part.material_hint.clone());
            let material = resolved_materials.entry(binding_key).or_insert_with(|| {
                material_for_part(
                    &part.binding,
                    part.material_hint.as_deref(),
                    inventory,
                    imported,
                    build,
                )
            });
            let path = material
                .as_ref()
                .map_err(Clone::clone)?
                .as_ref()
                .ok_or("No explicitly associated or same-stem MTL")?
                .clone();
            let authored = parsed_materials
                .entry(path.clone())
                .or_insert_with(|| {
                    read_material(&path).map_err(|e| format!("{}: {e}", path.display()))
                })
                .as_ref()
                .map_err(Clone::clone)?;
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
            let slot_key = (path.clone(), part.binding.clone(), part.material_slot);
            let maps = decoded_slots.entry(slot_key).or_insert_with(|| {
                authored_maps(
                    authored,
                    &path,
                    &part.binding,
                    part.material_slot,
                    MaterialLookup {
                        inventory,
                        imported,
                        build,
                    },
                    cache,
                    &mut budget,
                    Some(kala_state),
                )
            });
            let ([diffuse, normal, emissive, specular, gloss, opacity], map_warnings) =
                maps.as_ref().map_err(Clone::clone)?.clone();
            warnings.extend(
                map_warnings
                    .into_iter()
                    .map(|w| format!("{} material slot {}: {w}", part.name, part.material_slot)),
            );
            surface.diffuse = diffuse;
            surface.normal = normal;
            surface.emissive = emissive;
            surface.emission_strength = leaf.glow_amount.unwrap_or(1.0).max(0.0);
            surface.emissive_uv = source_decal_motion(leaf);
            // A single mesh MTL contains BOTH original Banshee blue and
            // Armor Reducer red leaves. Render only the active mode's
            // emissive tube; do not alter its normal/diffuse/UV0 sources.
            if kala_tubing_leaf(&path, &part.binding, leaf)
                && ((leaf.name.eq_ignore_ascii_case("armorreducer glow")
                    && kala_state == evolve_core::KalaTubingState::Banshee)
                    || (leaf.name.eq_ignore_ascii_case("bansheegun glow")
                        && kala_state == evolve_core::KalaTubingState::ArmorReducer))
            {
                surface.emissive = None;
                surface.emission_strength = 0.0;
                surface.emissive_uv = None;
            }
            // Authored 100/3 calibration is global in the GPU viewer.
            // Apply Kala's extra 0.3x/4x only to its WEAPON glow leaves.
            // Body/waist Glow stays source blue for every equipment choice.
            surface.kala_emission_state = (surface.emissive.is_some()
                && surface.emission_strength > 0.0
                && kala_tubing_leaf(&path, &part.binding, leaf)
                && !leaf.name.eq_ignore_ascii_case("Glow"))
            .then_some(kala_state);
            surface.specular = specular;
            surface.gloss = gloss;
            surface.foliage_cutout = source_vegetation_cutout(leaf) && surface.diffuse.is_some();
            surface.alpha_cutoff = leaf.alpha_test.unwrap_or(0.5);
            surface.opacity = if surface.foliage_cutout {
                None
            } else {
                verified_opacity(leaf, opacity)
            };
            surface.transparent =
                !surface.foliage_cutout && needs_alpha_blending(leaf) && surface.opacity.is_some();
            Ok(())
        })();
        if let Err(error) = result {
            warnings.push(format!("{} material: {}", part.name, error));
        }
        surfaces.push(surface);
        if let Some(progress) = progress {
            progress.store(
                35 + 50 * (index + 1) / report.source_parts.len(),
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    }
    // Goliath repeats one authored damage-blend warning across multiple
    // mesh ranges. Report each diagnostic once without truncating distinct
    // missing textures or unsafe MTL associations.
    let mut unique = HashSet::new();
    warnings.retain(|message| unique.insert(message.clone()));
    warn_gpu_material_limit(&surfaces, &mut warnings);
    (surfaces, warnings)
}

/// CryEngine *_glow_diff contains alternate armor colors and highlights.
/// Add only positive color differences from the original body in linear
/// light, without clipping each encoded channel at 80/255. This remains
/// an approximation, never an edit of source DDS, UV0 or FBX data.
pub fn armor_emission(
    base: &image::RgbaImage,
    glow: &image::RgbaImage,
) -> Result<image::RgbaImage, String> {
    if base.width() == 0
        || base.height() == 0
        || glow.width() == 0
        || glow.height() == 0
        || u64::from(base.width()) * u64::from(base.height()) > 32_000_000
    {
        return Err("Invalid armor glow/source diffuse dimensions".into());
    }
    let mut result = image::RgbaImage::new(base.width(), base.height());
    for (x, y, pixel) in result.enumerate_pixels_mut() {
        let sample = glow.get_pixel(
            (u64::from(x) * u64::from(glow.width()) / u64::from(base.width())) as u32,
            (u64::from(y) * u64::from(glow.height()) / u64::from(base.height())) as u32,
        );
        let normal = base.get_pixel(x, y);
        for c in 0..3 {
            pixel[c] = linear_srgb((srgb_linear(sample[c]) - srgb_linear(normal[c])).max(0.0));
        }
        pixel[3] = 255;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slider_reblends_loaded_surfaces_without_redecoding_or_changing_geometry() {
        let base_pixels = Arc::new(image::RgbaImage::from_pixel(
            2,
            1,
            image::Rgba([20, 50, 80, 255]),
        ));
        let full_pixels = Arc::new(image::RgbaImage::from_pixel(
            2,
            1,
            image::Rgba([220, 150, 80, 255]),
        ));
        let surface = Surface {
            first_index: 0,
            index_count: 3,
            visible: true,
            transparent: false,
            foliage_cutout: false,
            alpha_cutoff: 0.5,
            diffuse: Some(Arc::clone(&base_pixels)),
            normal: None,
            emissive: None,
            armor_emissive: None,
            emission_strength: 1.0,
            kala_emission_state: None,
            emissive_uv: None,
            specular: None,
            gloss: None,
            opacity: None,
        };
        let mut full = surface.clone();
        full.diffuse = Some(Arc::clone(&full_pixels));
        let half = blend_prepared_damage(
            std::slice::from_ref(&surface),
            std::slice::from_ref(&full),
            50,
        )
        .unwrap();
        assert_eq!(
            half[0].diffuse.as_ref().unwrap().get_pixel(0, 0).0,
            [120, 100, 80, 255]
        );
        assert_eq!(half[0].first_index, 0);
        assert_eq!(half[0].index_count, 3);
        assert!(Arc::ptr_eq(
            blend_prepared_damage(
                std::slice::from_ref(&surface),
                std::slice::from_ref(&full),
                0
            )
            .unwrap()[0]
                .diffuse
                .as_ref()
                .unwrap(),
            &base_pixels
        ));
        assert!(Arc::ptr_eq(
            blend_prepared_damage(
                std::slice::from_ref(&surface),
                std::slice::from_ref(&full),
                100
            )
            .unwrap()[0]
                .diffuse
                .as_ref()
                .unwrap(),
            &full_pixels
        ));
        assert!(blend_prepared_damage(&[surface], &[], 50).is_err());
    }

    #[test]
    fn alternate_armor_diffuse_emission_preserves_base_and_linear_highlights() {
        let base = image::RgbaImage::from_pixel(2, 1, image::Rgba([70, 45, 30, 255]));
        let glow =
            image::RgbaImage::from_raw(2, 1, vec![250, 200, 150, 255, 50, 30, 20, 255]).unwrap();
        let emission = armor_emission(&base, &glow).unwrap();
        assert!(emission.get_pixel(0, 0)[0] > 180);
        assert!(emission.get_pixel(0, 0)[1] > 130);
        assert!(emission.get_pixel(0, 0)[2] > 100);
        assert_eq!(emission.get_pixel(1, 0).0, [0, 0, 0, 255]);
        assert_eq!(base.get_pixel(0, 0).0, [70, 45, 30, 255]);
    }

    #[test]
    fn legacy_alpha_emission_requires_positive_authored_amount_and_real_mask() {
        let diffuse =
            image::RgbaImage::from_raw(2, 1, vec![255, 80, 30, 128, 10, 10, 10, 0]).unwrap();
        let mut slot = evolve_core::MaterialSlot {
            name: "glow".into(),
            shader: "Illum".into(),
            maps: vec![],
            opacity: None,
            alpha_test: None,
            glow_amount: Some(3.0),
            alpha_glow: true,
            diffuse_tint: None,
            decal_tex_mod: None,
        };
        let mask = legacy_alpha_emission(&slot, &diffuse).unwrap();
        assert!(mask.get_pixel(0, 0)[0] > 150);
        assert_eq!(mask.get_pixel(1, 0).0, [0, 0, 0, 255]);
        slot.glow_amount = Some(0.0);
        assert!(legacy_alpha_emission(&slot, &diffuse).is_none());
        slot.glow_amount = None;
        assert!(legacy_alpha_emission(&slot, &diffuse).is_none());
        slot.glow_amount = Some(3.0);
        slot.alpha_glow = false;
        assert!(legacy_alpha_emission(&slot, &diffuse).is_none());
        slot.alpha_glow = true;
        let opaque = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
        assert!(legacy_alpha_emission(&slot, &opaque).is_none());
    }

    #[test]
    fn grounder_illum_body_does_not_disappear_from_an_unverified_opacity_channel() {
        let plain = evolve_core::parse_material(
            br#"<Material Name="Grounder_Body" Shader="Illum" Opacity="1">
                <Textures>
                    <Texture Map="Diffuse" File="characters/monsters/grounder/grounder3_diff.tif"/>
                    <Texture Map="Opacity" File="characters/monsters/grounder/grounder3_detail.tif"/>
                </Textures>
            </Material>"#,
        )
        .unwrap();
        assert!(unverified_solid_body_opacity(&plain.slots[0]));
        let verified = evolve_core::parse_material(
            br#"<Material Name="MaskedFin" Shader="Illum">
                <Textures><Texture Map="Opacity" File="fin_cutout.tif"/></Textures>
            </Material>"#,
        )
        .unwrap();
        assert!(!unverified_solid_body_opacity(&verified.slots[0]));
        let glass = evolve_core::parse_material(
            br#"<Material Name="Glass" Shader="Glass">
                <Textures><Texture Map="Opacity" File="glass_opacity.tif"/></Textures>
            </Material>"#,
        )
        .unwrap();
        assert!(!unverified_solid_body_opacity(&glass.slots[0]));
    }

    #[test]
    fn kala_body_glow_remains_authored_blue_even_when_armor_reducer_is_selected() {
        let body = evolve_core::parse_material(
            br#"<Material Name="Glow" Shader="Illum" GlowAmount="3"
                Diffuse="0.072271854,0.1470273,0.42326769"><Textures>
                <Texture Map="Decal" File="textures/particle/light/chroma_fan320tiled_01b.tif"/>
                </Textures></Material>"#,
        )
        .unwrap();
        let body_slot = &body.slots[0];
        assert_eq!(body_slot.glow_amount, Some(3.0));
        assert!(kala_tubing_leaf(
            Path::new("characters/hunters/merc_tech/tech_3p.mtl"),
            "characters/hunters/merc_tech/tech_3p.skin",
            body_slot
        ));
        let original = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        let output = tint_legacy_decal_for_state(body_slot, &original, true);
        let blue = output.get_pixel(0, 0);
        assert!(blue[2] > blue[1] && blue[1] > blue[0]);
        assert_eq!(original.get_pixel(0, 0).0, [255, 0, 0, 255]);
        // Red is legitimately sourced ONLY from a distinct equipment leaf.
        let reducer = evolve_core::parse_material(
            br#"<Material Name="armorreducer glow" Shader="Illum" GlowAmount="3"
                Diffuse="0.17788844,0.0074990322,0.0043914425"><Textures>
                <Texture Map="Decal" File="textures/particle/light/chroma_fan320tiled_01b.tif"/>
                </Textures></Material>"#,
        )
        .unwrap();
        let red = tint_legacy_decal_for_state(&reducer.slots[0], &original, true);
        let pixel = red.get_pixel(0, 0);
        assert!(pixel[0] > pixel[1] && pixel[0] > pixel[2]);
        assert!(kala_tubing_leaf(
            Path::new("characters/hunters/merc_tech/weapons/bansheegun/techbansheegun_3p.mtl"),
            "characters/hunters/merc_tech/techbansheegun_3p.skin",
            &reducer.slots[0]
        ));
    }

    #[test]
    fn kala_real_banshee_glow_decal_is_not_zeroed_by_body_alpha() {
        let document = evolve_core::parse_material(
            br#"<Material Name="bansheegun glow" Shader="Illum"
                StringGenMask="%SUBSURFACE_SCATTERING"
                Diffuse="0.072271854,0.1470273,0.42326769"
                Emissive="0,0,0" GlowAmount="3"><Textures>
                <Texture Map="Diffuse" File="characters/hunters/merc_tech/body.tif"/>
                <Texture Map="Decal" File="textures/particle/light/chroma_fan320tiled_01b.tif">
                    <TexMod TileU="3" TileV="4"/>
                </Texture>
            </Textures></Material>"#,
        )
        .unwrap();
        let slot = &document.slots[0];
        assert!(!slot.alpha_glow);
        assert_eq!(
            slot.decal_tex_mod.unwrap().sample(0.0),
            [3.0, 4.0, 0.0, 0.0]
        );
        assert_eq!(legacy_decal_role(slot, "decal"), Some(2));
        assert_eq!(legacy_decal_role(slot, "normals"), Some(1));
        let original = image::RgbaImage::from_pixel(2, 2, image::Rgba([180, 180, 180, 220]));
        let decal = tint_legacy_decal(slot, &original);
        assert!(decal.pixels().all(|p| p[2] > p[1] && p[1] > p[0]));
        assert!(decal.pixels().all(|p| p[2] > 0));
        assert_eq!(original.get_pixel(0, 0)[0], 180);
        assert!(kala_banshee_brightness_mask(slot));

        // Regression: a red frame in the original rainbow/chroma atlas
        // must still generate BLUE light on Kala, without modifying UV motion.
        let red_frame = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        let corrected = tint_legacy_decal(slot, &red_frame);
        let color = corrected.get_pixel(0, 0);
        assert!(color[2] > color[1] && color[1] > color[0]);
        assert!(color[2] > 0);
        assert_eq!(*red_frame.get_pixel(0, 0), image::Rgba([255, 0, 0, 255]));
        assert_eq!(slot.decal_tex_mod.unwrap().sample(2.0)[..2], [3.0, 4.0]);

        // Atlas alpha remains an intensity mask, not mesh opacity.
        let transparent = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 0]));
        assert_eq!(
            tint_legacy_decal(slot, &transparent).get_pixel(0, 0).0,
            [0, 0, 0, 255]
        );
        let mut unrelated = slot.clone();
        unrelated.name = "other legacy emissive".into();
        assert!(!kala_banshee_brightness_mask(&unrelated));
        let preserved = tint_legacy_decal(&unrelated, &red_frame);
        assert!(preserved.get_pixel(0, 0)[0] > preserved.get_pixel(0, 0)[2]);

        let mut body = slot.clone();
        body.alpha_glow = true;
        assert_eq!(legacy_decal_role(&body, "decal"), None);
        let disabled = evolve_core::parse_material(
            br#"<Material Shader="Illum" GlowAmount="0"><Textures>
                <Texture Map="Decal" File="textures/particle/light/chroma_fan320tiled_01b.tif"/>
            </Textures></Material>"#,
        )
        .unwrap();
        assert_eq!(legacy_decal_role(&disabled.slots[0], "decal"), None);
    }

    #[test]
    fn kala_banshee_emissive_roles_and_source_zero_are_distinct() {
        assert_eq!(role_channel("emissivecolor"), Some(2));
        assert_eq!(role_channel("glowmap"), Some(2));
        assert_eq!(role_channel("selfillumination"), Some(2));
        assert_eq!(role_channel("normals"), Some(1));
        assert_eq!(role_channel("custom"), None);
        let authored = evolve_core::parse_material(
            br#"<Material Name="BansheeGun" Shader="Illum" GlowAmount="0"><Textures>
                <Texture Map="EmissiveColor" File="characters/hunters/tech/banshee_glow.tif"/>
            </Textures></Material>"#,
        )
        .unwrap();
        assert_eq!(authored.slots[0].glow_amount, Some(0.0));
        assert_eq!(role_channel(&authored.slots[0].maps[0].role), Some(2));
    }

    #[test]
    fn vegetation_shader_diffuse_alpha_is_cutout_not_foliage_transmittance() {
        let doc = evolve_core::parse_material(
            br#"<Material Name="Leaves" Shader="Vegetation" Opacity="1">
                    <Textures>
                        <Texture Map="Diffuse" File="objects/forest/leaves_diff.tif"/>
                        <Texture Map="Opacity" File="objects/forest/leaves_sss.tif"/>
                    </Textures>
                </Material>"#,
        )
        .unwrap();
        let leaf = &doc.slots[0];
        assert!(source_vegetation_cutout(leaf));
        assert!(!needs_alpha_blending(leaf));
        // CryEngine uses the opacity map for light transmission through
        // foliage; red/alpha must not drive silhouette rejection.
        assert!(!unverified_solid_body_opacity(leaf));
        let ordinary = evolve_core::parse_material(
            br#"<Material Name="Body" Shader="Illum">
                    <Textures><Texture Map="Diffuse" File="body.tif"/></Textures>
                </Material>"#,
        )
        .unwrap();
        assert!(!source_vegetation_cutout(&ordinary.slots[0]));
        let translucent =
            evolve_core::parse_material(br#"<Material Shader="Vegetation" Opacity="0.5"/>"#)
                .unwrap();
        assert!(!source_vegetation_cutout(&translucent.slots[0]));
    }

    #[test]
    fn solid_character_opacity_mask_is_depth_opaque_not_alpha_blended() {
        let make = |shader: &str, scalar: Option<f32>, with_mask: bool| evolve_core::MaterialSlot {
            name: "Goliath_Body".into(),
            shader: shader.into(),
            opacity: scalar,
            alpha_test: None,
            glow_amount: None,
            alpha_glow: false,
            diffuse_tint: None,
            decal_tex_mod: None,
            maps: if with_mask {
                vec![evolve_core::TextureReference {
                    role: "opacity".into(),
                    path: "characters/monsters/goliath/body_opacity.tif".into(),
                }]
            } else {
                vec![]
            },
        };
        assert!(!needs_alpha_blending(&make("Illum", Some(1.0), true)));
        assert!(!needs_alpha_blending(&make("HumanSkin", Some(0.7), true)));
        assert!(!needs_alpha_blending(&make("Illum", None, false)));
        assert!(needs_alpha_blending(&make("Glass", Some(1.0), true)));
        assert!(needs_alpha_blending(&make("Water", None, false)));
        assert!(needs_alpha_blending(&make("Illum", Some(0.35), false)));
    }

    #[test]
    fn goliath3_damage_blend_opacity_preserves_solid_body() {
        // Source shape: Goliath3.mtl uses one Illum mesh slot with
        // goliath1_damage_blend_diff.tif in its "Opacity" channel, while
        // separate NoDraw skeleton slots and eye slot remain in place.
        let document = evolve_core::parse_material(
            br#"<Material><SubMaterials>
                <Material Name="head_skel" Shader="NoDraw"/>
                <Material Name="torso_skel" Shader="NoDraw"/>
                <Material Name="limbs_skel" Shader="NoDraw"/>
                <Material Name="mesh" Shader="Illum" Opacity="1">
                    <Textures>
                        <Texture Map="Diffuse" File="characters/monsters/goliath/textures/goliath3_diff.tif"/>
                        <Texture Map="Opacity" File="characters/monsters/goliath/textures/goliath1_damage_blend_diff.tif"/>
                    </Textures>
                </Material>
                <Material Name="unused" Shader="NoDraw"/>
                <Material Name="Eyes" Shader="Illum">
                    <Textures><Texture Map="Diffuse" File="characters/monsters/goliath/textures/goliath3_diff.tif"/></Textures>
                </Material>
            </SubMaterials></Material>"#,
        )
        .unwrap();
        assert_eq!(document.slots.len(), 6);
        let body = &document.slots[3];
        let damage = body.maps.iter().find(|map| map.role == "opacity").unwrap();
        assert!(is_damage_blend_mask(body, damage));
        assert!(!needs_alpha_blending(body));
        assert!(verified_opacity(body, None).is_none());
        assert_eq!(document.slots[5].name, "Eyes");

        // A real authored cutout must still retain its opacity semantics.
        let genuine = evolve_core::parse_material(
            br#"<Material Shader="Illum"><Textures>
                <Texture Map="Opacity" File="textures/leaf_cutout.tif"/>
            </Textures></Material>"#,
        )
        .unwrap();
        assert!(!is_damage_blend_mask(
            &genuine.slots[0],
            &genuine.slots[0].maps[0]
        ));
    }

    #[test]
    fn character_relative_dds0_loading_matches_private_material_scoping() {
        let root = tempfile::tempdir().unwrap();
        let owner = root.path().join("characters/monsters/teleporter");
        std::fs::create_dir_all(owner.join("materials")).unwrap();
        std::fs::create_dir_all(owner.join("textures")).unwrap();
        let material = owner.join("materials/body.mtl");
        std::fs::write(
            &material,
            br#"<Material><Textures><Texture Map="Diffuse" File="body.tif"/></Textures></Material>"#,
        )
        .unwrap();
        let mut dds = vec![0u8; 128 + 8];
        dds[..4].copy_from_slice(b"DDS ");
        dds[4..8].copy_from_slice(&124u32.to_le_bytes());
        dds[8..12].copy_from_slice(&0x1007u32.to_le_bytes());
        dds[12..16].copy_from_slice(&4u32.to_le_bytes());
        dds[16..20].copy_from_slice(&4u32.to_le_bytes());
        dds[76..80].copy_from_slice(&32u32.to_le_bytes());
        dds[80..84].copy_from_slice(&4u32.to_le_bytes());
        dds[84..88].copy_from_slice(b"DXT1");
        dds[128..136].copy_from_slice(&[0, 248, 0, 0, 0, 0, 0, 0]);
        std::fs::write(owner.join("textures/body.dds.0"), dds).unwrap();
        let inventory = evolve_core::scan_directory(root.path()).unwrap();
        let document = read_material(&material).unwrap();
        let mut budget = TextureBudget::default();
        let (maps, diagnostics) = authored_maps(
            &document,
            &material,
            "characters/monsters/teleporter/body.skinm",
            0,
            MaterialLookup {
                inventory: &inventory,
                imported: None,
                build: None,
            },
            &SharedCache::default(),
            &mut budget,
            None,
        )
        .unwrap();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(maps[0].as_ref().unwrap().dimensions(), (4, 4));
        assert!(maps[0].as_ref().unwrap().get_pixel(0, 0).0[0] > 200);
    }

    #[test]
    fn original_texture_scopes_reject_other_actor_basename_and_bad_paths() {
        let owner = "characters/monsters/teleporter/materials/body.mtl";
        let scoped = scoped_texture_candidates("body.tif", owner).unwrap();
        assert_eq!(
            scoped[0],
            "characters/monsters/teleporter/materials/body.dds.0"
        );
        assert!(scoped.contains(&"characters/monsters/teleporter/textures/body.dds.0".to_owned()));
        assert!(scoped.contains(&"characters/monsters/shared/textures/body.dds.0".to_owned()));
        assert!(!scoped.iter().any(|path| path.contains("other_monster")));
        assert!(scoped_texture_candidates("../body.tif", owner).is_err());
        assert!(scoped_texture_candidates("C:/body.tif", owner).is_err());
    }

    #[test]
    fn shared_dds_counts_once_per_gpu_role_not_per_cdf_attachment() {
        let image = Arc::new(image::RgbaImage::new(4, 4));
        let surface = Surface {
            first_index: 0,
            index_count: 3,
            visible: true,
            transparent: false,
            foliage_cutout: false,
            alpha_cutoff: 0.5,
            diffuse: Some(Arc::clone(&image)),
            normal: None,
            emissive: None,
            armor_emissive: None,
            emission_strength: 1.0,
            kala_emission_state: None,
            emissive_uv: None,
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
    fn historical_goliath_variant_material_can_be_resolved_without_other_character_fallbacks() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Objects/characters/monsters/Goliath_2");
        std::fs::create_dir_all(&folder).unwrap();
        let mut source = vec![0; 20];
        source[..8].copy_from_slice(b"CryTek\0\0");
        source[8..12].copy_from_slice(&0xffff0000u32.to_le_bytes());
        source[12..16].copy_from_slice(&0x745u32.to_le_bytes());
        std::fs::write(folder.join("goliath3_default_head.chr"), &source).unwrap();
        std::fs::write(
            folder.join("goliath3.mtl"),
            br#"<Material Name="Goliath"/>"#,
        )
        .unwrap();
        std::fs::write(folder.join("goliath2.mtl"), br#"<Material Name="Wrong"/>"#).unwrap();
        let inventory = evolve_core::scan_directory(dir.path()).unwrap();
        let binding = "Objects/characters/monsters/Goliath_2/goliath3_default_head.chr";
        let found = material_for_part(binding, None, &inventory, None, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            found.file_name().and_then(|n| n.to_str()),
            Some("goliath3.mtl")
        );
        let explicit = material_for_part(
            binding,
            Some("Objects/characters/monsters/Goliath_2/goliath3"),
            &inventory,
            None,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(explicit, found);
        let scoped = material_for_part(binding, Some("goliath3"), &inventory, None, None)
            .unwrap()
            .unwrap();
        assert_eq!(scoped, found);
        assert!(material_for_part(binding, Some("goliath2.exr"), &inventory, None, None).is_err());
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
    fn goliath_material_ranges_keep_distinct_authored_high_mip_colors() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("goliath3.skinm");
        let material = dir.path().join("goliath3.mtl");
        std::fs::write(&model, compiled_material_fixture(true)).unwrap();
        std::fs::write(
            &material,
            br#"<Material Name="Goliath"><SubMaterials>
                <Material Name="Goliath_Body" Shader="Illum"><Textures>
                    <Texture Map="Diffuse" File="body.tif"/>
                </Textures></Material>
                <Material Name="Goliath_Arm" Shader="Illum"><Textures>
                    <Texture Map="Diffuse" File="arm.tif"/>
                </Textures></Material>
            </SubMaterials></Material>"#,
        )
        .unwrap();
        let make_dds = |red: bool| {
            let mut data = vec![0u8; 136];
            data[..4].copy_from_slice(b"DDS ");
            data[4..8].copy_from_slice(&124u32.to_le_bytes());
            data[8..12].copy_from_slice(&0x1007u32.to_le_bytes());
            data[12..16].copy_from_slice(&4u32.to_le_bytes());
            data[16..20].copy_from_slice(&4u32.to_le_bytes());
            data[76..80].copy_from_slice(&32u32.to_le_bytes());
            data[80..84].copy_from_slice(&4u32.to_le_bytes());
            data[84..88].copy_from_slice(b"DXT1");
            let color = if red { [0, 248] } else { [31, 0] };
            data[128..130].copy_from_slice(&color);
            data
        };
        std::fs::write(dir.path().join("body.dds.0"), make_dds(true)).unwrap();
        std::fs::write(dir.path().join("arm.dds.0"), make_dds(false)).unwrap();
        let inventory = evolve_core::scan_directory(dir.path()).unwrap();
        let (surfaces, warnings) = load_model(
            &model,
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
        let body = surfaces[0].diffuse.as_ref().unwrap();
        let arm = surfaces[1].diffuse.as_ref().unwrap();
        assert_ne!(body.get_pixel(0, 0)[0], arm.get_pixel(0, 0)[0]);
        assert!(body.get_pixel(0, 0)[0] > body.get_pixel(0, 0)[2]);
        assert!(arm.get_pixel(0, 0)[2] > arm.get_pixel(0, 0)[0]);
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
            &dir.path().join("armor.mtl"),
            "armor.mtl",
            0,
            MaterialLookup {
                inventory: &inventory,
                imported: None,
                build: None,
            },
            &SharedCache::default(),
            &mut budget,
            None,
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
        // Authored opacity masks on solid shaders now use depth-writing
        // alpha test, unlike genuinely translucent glass/water surfaces.
        assert!(!needs_alpha_blending(slot));
        assert!(
            slot.maps
                .iter()
                .any(|reference| reference.role == "opacity")
        );
        assert!(!is_translucent_shader(slot));
    }

    #[test]
    fn authored_opacity_scalars_and_glass_do_not_consume_diffuse_alpha() {
        let solid =
            evolve_core::parse_material(br#"<Material Shader="Illum" Opacity="1"/>"#).unwrap();
        assert!(verified_opacity(&solid.slots[0], None).is_none());
        assert!(!needs_alpha_blending(&solid.slots[0]));
        let fading =
            evolve_core::parse_material(br#"<Material Shader="Illum" Opacity="0.11"/>"#).unwrap();
        let alpha = verified_opacity(&fading.slots[0], None).unwrap();
        assert_eq!(alpha.get_pixel(0, 0).0[0], 28);
        assert!(needs_alpha_blending(&fading.slots[0]));
        let glass =
            evolve_core::parse_material(br#"<Material Shader="Glass" Opacity="1"/>"#).unwrap();
        assert!(needs_alpha_blending(&glass.slots[0]));
        let neutral = verified_opacity(&glass.slots[0], None).unwrap();
        assert_eq!(neutral.get_pixel(0, 0).0[0], 102);

        let source = Arc::new(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([128, 64, 16, 0]),
        ));
        let scaled = verified_opacity(&fading.slots[0], Some(source.clone())).unwrap();
        assert_eq!(scaled.get_pixel(1, 1).0[0], 14);
        assert_eq!(source.get_pixel(1, 1).0[0], 128);
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
            opacity: None,
            alpha_test: None,
            glow_amount: None,
            alpha_glow: false,
            diffuse_tint: None,
            decal_tex_mod: None,
        };
        assert!(!needs_alpha_blending(&ordinary));
        let mut transparent = ordinary;
        transparent.maps.push(evolve_core::TextureReference {
            role: "opacity".into(),
            path: "characters/glass_mask.dds".into(),
        });
        assert!(!needs_alpha_blending(&transparent));
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
