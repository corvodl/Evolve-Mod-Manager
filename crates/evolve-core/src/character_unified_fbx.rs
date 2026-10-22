//! Opt-in unified CDF character FBX, using original CryEngine skeleton/weights.
//! Fail closed on any missing part, UV, mismatched rig or invalid Blender output.
//! This is geometry/rig unification; full multi-slot materials remain a separate gate.
use crate::cdf::{AssetResolution, read_cdf, resolve_cdf, resolve_virtual};
use crate::character_assembly::select_attachments;
use crate::character_equipment::{EquipmentOverrides, validated_equipment_overrides};
use crate::fbx::publish_verified_binary_fbx_with_normals;
use crate::inventory::Inventory;
use crate::library_catalog::{
    LibraryReferenceStatus, imported_library, resolve_imported_reference,
    resolve_imported_reference_in_archive,
};
use crate::materials::{read_material, scoped_texture_candidates};
use crate::model::{has_original_mesh_subset_table, original_mesh_subsets, read_model};
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
    /// Number of verified original armatures in the FBX. A historical 2012
    /// character can preserve several incompatible source rest skeletons
    /// without silently dropping the extra joint or reassigning weights.
    #[serde(default)]
    pub armatures: usize,
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

type ResolvedCdfParts = (
    PathBuf,
    String,
    Option<String>,
    Vec<(String, PathBuf, Option<String>)>,
);

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
    let selected =
        crate::character_emet::choose(&actual.to_string_lossy(), &definition, selected, equipment)?;
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
        parts.push((name, compiled, part.material.clone()));
    }
    if parts.is_empty() || parts.len() != selected.len() || parts.len() > MAX_PARTS {
        return Err("CDF has incomplete or excessive selected attachments".into());
    }
    Ok((skeleton, skeleton_ref, skin_set, parts))
}

const SOURCE_EMISSION_ROLES: &[&str] = &[
    "emissive",
    "emittance",
    "emission",
    "glow",
    "emissivecolor",
    "emissionmap",
    "glowmap",
    "selfillum",
    "selfillumination",
];

#[derive(Default)]
struct AuthoredMaps {
    diffuse: Option<PathBuf>,
    normal: Option<PathBuf>,
    emissive: Option<PathBuf>,
    specular: Option<PathBuf>,
    gloss: Option<PathBuf>,
}

/// Resolve other imported archives only when the source is an actual
/// imported Asset Library PAK. The source archive's unique build label
/// constrains lookups so Stage 2 and 2012 textures cannot be mixed.
struct ImportedTextureScope {
    root: PathBuf,
    build: Option<String>,
}

fn texture_scope(inventory: &Inventory) -> Option<ImportedTextureScope> {
    let files = fs::canonicalize(&inventory.root).ok()?;
    if files.file_name()?.to_str()? != "files" {
        return None;
    }
    let archive = files.parent()?;
    let library_root = archive.parent()?;
    let catalog = imported_library(library_root).ok()?;
    let source = catalog
        .archives
        .iter()
        .find(|record| fs::canonicalize(&record.directory).is_ok_and(|path| path == archive))?;
    // If the source belongs to multiple builds, do not guess a game version.
    let build = (source.builds.len() == 1).then(|| source.builds[0].clone());
    Some(ImportedTextureScope {
        root: library_root.to_path_buf(),
        build,
    })
}

/// The FBX exporter must use the SAME authored MTL ownership search as
/// the live GPU viewer, not an unrelated global texture with the right name.
fn exact_original_texture(
    root: &Path,
    inventory: &Inventory,
    material_virtual: &str,
    authored: &str,
    imported: Option<&ImportedTextureScope>,
) -> Result<Option<PathBuf>, String> {
    for candidate in scoped_texture_candidates(authored, material_virtual)? {
        match resolve_virtual(inventory, &candidate) {
            AssetResolution::Found(path) => return input_file(root, &path).map(Some),
            AssetResolution::Missing => {}
            AssetResolution::Ambiguous(_) => {
                return Err(format!(
                    "Ambiguous source texture {candidate} in {material_virtual}"
                ));
            }
        }
        if let Some(scope) = imported {
            match resolve_imported_reference(&scope.root, &candidate, scope.build.as_deref())
                .map_err(|error| format!("Cannot search imported texture {candidate}: {error}"))?
            {
                LibraryReferenceStatus::Found(hit) => {
                    // The library resolver checks manifest membership, size,
                    // symlink boundaries, original archive and build identity.
                    // Keep a final bounded read guard for the FBX encoder.
                    let path = PathBuf::from(hit.absolute_path);
                    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                    if !metadata.file_type().is_file()
                        || metadata.len() == 0
                        || metadata.len() > MAX_SOURCE
                    {
                        return Err(format!("Unsafe imported source texture {candidate}"));
                    }
                    return fs::canonicalize(path).map(Some).map_err(|e| e.to_string());
                }
                LibraryReferenceStatus::Missing => {}
                LibraryReferenceStatus::Ambiguous(_) => {
                    return Err(format!(
                        "Multiple imported sources match {candidate}; choose one source build before FBX export"
                    ));
                }
            }
        }
    }
    Ok(None)
}

/// Find only immediately adjacent missing high mip indices for a DDS.0
/// from an imported, uniquely build-qualified archive. Stage 2 sometimes
/// splits the same original texture between dds_low.pak (.0..4) and
/// dds_high.pak (.5). Reuse the verified Library manifest; never guess
/// unindexed content, other builds, or missing middle fragments.
fn matching_imported_high_mips(
    file: &Path,
    scope: Option<&ImportedTextureScope>,
) -> Result<Vec<PathBuf>, String> {
    let Some(scope) = scope else {
        return Ok(Vec::new());
    };
    if scope.build.is_none() {
        // A unique build is essential before stitching across archives.
        return Ok(Vec::new());
    }
    let real = fs::canonicalize(file).map_err(|e| e.to_string())?;
    let library_root = fs::canonicalize(&scope.root).map_err(|e| e.to_string())?;
    let Some(files) = real.ancestors().find(|candidate| {
        candidate.file_name().is_some_and(|name| name == "files")
            && candidate.parent().and_then(Path::parent) == Some(library_root.as_path())
    }) else {
        return Ok(Vec::new());
    };
    let relative = real.strip_prefix(files).map_err(|e| e.to_string())?;
    let Some(filename) = relative.file_name().and_then(|name| name.to_str()) else {
        return Ok(Vec::new());
    };
    let lowercase = filename.to_ascii_lowercase();
    let Some((stem, number)) = lowercase.rsplit_once(".dds.") else {
        return Ok(Vec::new());
    };
    if number != "0" {
        return Ok(Vec::new());
    }
    let Some(parent) = real.parent() else {
        return Ok(Vec::new());
    };
    let mut highest = 0usize;
    for item in fs::read_dir(parent).map_err(|e| e.to_string())? {
        let item = item.map_err(|e| e.to_string())?;
        let filename = item.file_name().to_string_lossy().to_ascii_lowercase();
        if let Some(suffix) = filename.strip_prefix(&format!("{stem}.dds."))
            && let Ok(index) = suffix.parse::<usize>()
        {
            highest = highest.max(index);
        }
    }
    let base = relative
        .with_file_name(format!("{stem}.dds"))
        .to_string_lossy()
        .replace('\\', "/");
    let mut found = Vec::new();
    for index in highest.saturating_add(1)..20 {
        let candidate = format!("{base}.{index}");
        match resolve_imported_reference(&scope.root, &candidate, scope.build.as_deref())
            .map_err(|e| format!("Cannot verify source DDS fragment {candidate}: {e}"))?
        {
            LibraryReferenceStatus::Found(hit) => {
                let path = PathBuf::from(hit.absolute_path);
                let original = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                if !original.file_type().is_file()
                    || original.len() == 0
                    || original.len() > MAX_SOURCE
                {
                    return Err(format!("Unsafe verified DDS fragment {candidate}"));
                }
                let canonical = fs::canonicalize(path).map_err(|e| e.to_string())?;
                if !canonical.starts_with(&library_root) {
                    return Err(format!(
                        "DDS fragment {candidate} escaped the imported library"
                    ));
                }
                found.push(canonical);
            }
            LibraryReferenceStatus::Missing => break,
            LibraryReferenceStatus::Ambiguous(_) => {
                return Err(format!(
                    "Multiple source-build matches for DDS fragment {candidate}"
                ));
            }
        }
    }
    Ok(found)
}

fn decode_original_mip(
    file: &Path,
    imported: Option<&ImportedTextureScope>,
) -> Result<(crate::dds::DdsInfo, image::RgbaImage), String> {
    match crate::dds::decode_dds_rgba(file) {
        Ok(result) => Ok(result),
        Err(first) => {
            let should_try = matches!(
                &first,
                crate::dds::DdsError::Unsupported(reason)
                    if reason.contains("incomplete or noncontiguous split DDS set")
            );
            if !should_try {
                return Err(format!(
                    "Cannot decode original DDS {}: {first}",
                    file.display()
                ));
            }
            let extra = matching_imported_high_mips(file, imported)?;
            if extra.is_empty() {
                return Err(format!(
                    "Original DDS {} needs missing high-mip fragments from its matching build's imported dds_high.pak: {first}",
                    file.display()
                ));
            }
            crate::dds::decode_dds_rgba_with_parts(file, &extra)
                .map_err(|e| format!("Cross-PAK original DDS {}: {e}", file.display()))
        }
    }
}

/// Source-exact full-armor monster glow. A glow asset must belong to the
/// selected CDF's character/stage directory, and must have an indexed,
/// manifest-verified texture (possibly from a matching imported DDS PAK).
/// It is never selected by a global filename search.
pub fn monster_glow_path(
    cdf_source: &Path,
    inventory: &Inventory,
) -> Result<Option<PathBuf>, String> {
    monster_glow_path_with_library(cdf_source, inventory, None, None)
}

/// Resolve the authored monster variant's optional glow DDS from either its
/// original indexed PAK or other *verified same-build* Asset Library PAKs.
/// The viewer already knows its imported Library root and selected build;
/// don't silently discard that context (the old lookup did, and missed most
/// externally stored dds_low/dds_high glow maps).
pub fn monster_glow_path_with_library(
    cdf_source: &Path,
    inventory: &Inventory,
    imported_library_root: Option<&Path>,
    selected_build: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let root = fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
    let source = fs::canonicalize(cdf_source).map_err(|e| e.to_string())?;
    let relative = source
        .strip_prefix(&root)
        .map_err(|_| "Monster CDF outside indexed assets")?;
    let virtual_cdf = relative.to_string_lossy().replace('\\', "/");
    let lower = virtual_cdf.to_ascii_lowercase();
    if !(lower.starts_with("characters/monsters/") || lower.contains("/characters/monsters/"))
        || !lower.ends_with(".cdf")
    {
        return Ok(None);
    }
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Invalid monster CDF filename")?;
    // Explicit caller context wins when present; the auto-detected archive
    // scope remains a fallback for existing FBX export callers.
    let inferred = texture_scope(inventory);
    let explicit = imported_library_root.map(|library_root| ImportedTextureScope {
        root: library_root.to_path_buf(),
        build: selected_build
            .map(str::to_owned)
            .or_else(|| inferred.as_ref().and_then(|scope| scope.build.clone())),
    });
    let imported = explicit.as_ref().or(inferred.as_ref());
    for suffix in ["_glow_diff.tif", "_glow.tif"] {
        let authored = format!("{stem}{suffix}");
        if let Some(found) =
            exact_original_texture(&root, inventory, &virtual_cdf, &authored, imported)?
        {
            return Ok(Some(found));
        }
    }
    // Stage 2's dedicated characters_monsters_<owner>_ts.pak often contains
    // flat names, e.g. grounder_glow_diff.dds.0 through .7, whereas the
    // selected CDF can be named grounder3.cdf. These are not global names:
    // only the original monster folder determines <owner>, and the fallback
    // searches that *exact* same-build monster texture archive.
    let Some(scope) = imported else {
        return Ok(None);
    };
    let Some(build) = scope.build.as_deref() else {
        // Never use a root-level filename without a selected/unique build.
        return Ok(None);
    };
    let Some((_, monster_relative)) = lower.split_once("characters/monsters/") else {
        return Ok(None);
    };
    let Some(owner) = monster_relative.split('/').next() else {
        return Ok(None);
    };
    if owner.is_empty()
        || !owner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Ok(None);
    }
    let archive = format!("characters_monsters_{owner}_ts.pak");
    let mut stems = vec![stem.to_ascii_lowercase()];
    if !stems.iter().any(|value| value == owner) {
        stems.push(owner.to_owned());
    }
    for name in stems {
        for suffix in ["_glow_diff.tif", "_glow.tif"] {
            let authored = format!("{name}{suffix}");
            match resolve_imported_reference_in_archive(&scope.root, &authored, build, &archive)
                .map_err(|error| format!("Cannot search {archive} for {authored}: {error}"))?
            {
                LibraryReferenceStatus::Found(hit) => {
                    let path = PathBuf::from(hit.absolute_path);
                    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                    if !metadata.file_type().is_file()
                        || metadata.len() == 0
                        || metadata.len() > MAX_SOURCE
                    {
                        return Err(format!("Unsafe monster glow texture {authored}"));
                    }
                    return fs::canonicalize(path).map(Some).map_err(|e| e.to_string());
                }
                LibraryReferenceStatus::Missing => {}
                LibraryReferenceStatus::Ambiguous(_) => {
                    return Err(format!(
                        "Ambiguous monster glow {authored} in the selected build's {archive}"
                    ));
                }
            }
        }
    }
    Ok(None)
}

/// Read the authored CryEngine CrChF / MtlName (0x08021014) reference.
/// The live GPU viewer already accepts this chunk, but CDF FBX export
/// previously ignored it and rejected multi-subset parts such as Emet's
/// `oh` attachment even with the original shared MTL present.
fn compiled_material_names(source: &[u8]) -> Result<Vec<String>, String> {
    if source.len() < 32 || source.get(..5) != Some(b"CrChF") {
        return Ok(Vec::new());
    }
    let value = |offset: usize| -> Option<usize> {
        Some(u32::from_le_bytes(source.get(offset..offset + 4)?.try_into().ok()?) as usize)
    };
    let (Some(count), Some(start)) = (value(8), value(12)) else {
        return Ok(Vec::new());
    };
    if count == 0
        || count > 10_000
        || start < 16
        || count
            .checked_mul(16)
            .and_then(|n| start.checked_add(n))
            .is_none_or(|end| end > source.len())
    {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for i in 0..count {
        let at = start + i * 16;
        if value(at) != Some(0x08021014) {
            continue;
        }
        let (Some(len), Some(offset)) = (value(at + 8), value(at + 12)) else {
            continue;
        };
        if len < 132
            || offset < start + count * 16
            || offset.checked_add(len).is_none_or(|end| end > source.len())
        {
            continue;
        }
        let Some(field) = source.get(offset..offset + 128) else {
            continue;
        };
        let Some(end) = field.iter().position(|byte| *byte == 0) else {
            continue;
        };
        let Ok(authored) = std::str::from_utf8(&field[..end]) else {
            continue;
        };
        let authored = authored.trim().replace('\\', "/");
        if authored.is_empty() {
            continue;
        }
        let authored = if authored.to_ascii_lowercase().ends_with(".mtl") {
            authored
        } else {
            format!("{authored}.mtl")
        };
        // Unlike a folder-wide filename search this is the exact authored
        // reference inside this particular render mesh.
        if !names
            .iter()
            .any(|name: &String| name.eq_ignore_ascii_case(&authored))
        {
            names.push(authored);
        }
    }
    Ok(names)
}

/// Locate an exact MTL from this source or its verified same-build library.
/// Never accept similarly named files outside the authored virtual path.
fn exact_authored_mtl(
    root: &Path,
    inventory: &Inventory,
    candidate: &str,
    imported: Option<&ImportedTextureScope>,
) -> Result<Option<(String, PathBuf)>, String> {
    match resolve_virtual(inventory, candidate) {
        AssetResolution::Found(relative) => {
            let path = input_file(root, &relative)?;
            return Ok(Some((relative, path)));
        }
        AssetResolution::Ambiguous(_) => {
            return Err(format!("Ambiguous original material reference {candidate}"));
        }
        AssetResolution::Missing => {}
    }
    let Some(scope) = imported else {
        return Ok(None);
    };
    // Cross-PAK reference only when the source archive is bound to a
    // single build: do not mix Stage 2, 2012 or other PAKs.
    let Some(build) = scope.build.as_deref() else {
        return Ok(None);
    };
    match resolve_imported_reference(&scope.root, candidate, Some(build))
        .map_err(|e| format!("Cannot resolve original MTL {candidate}: {e}"))?
    {
        LibraryReferenceStatus::Found(hit) => {
            let path = PathBuf::from(hit.absolute_path);
            let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            let real = fs::canonicalize(&path).map_err(|e| e.to_string())?;
            let library = fs::canonicalize(&scope.root).map_err(|e| e.to_string())?;
            if !meta.file_type().is_file()
                || meta.len() == 0
                || meta.len() > MAX_SOURCE
                || !real.starts_with(library)
            {
                return Err(format!("Unsafe indexed material {candidate}"));
            }
            Ok(Some((candidate.to_owned(), real)))
        }
        LibraryReferenceStatus::Missing => Ok(None),
        LibraryReferenceStatus::Ambiguous(_) => {
            Err(format!("Ambiguous same-build original MTL {candidate}"))
        }
    }
}

/// A CDF can bind Scorpid3_Rig.chr while its original MTL is
/// Scorpid3.mtl. Authored overrides outrank embedded MtlName, which outranks
/// matching filenames. Never use a different character's material.
#[cfg(test)]
fn original_model_material(
    root: &Path,
    mesh: &Path,
    inventory: &Inventory,
    override_mtl: Option<&str>,
) -> Result<Option<(String, PathBuf)>, String> {
    original_model_material_with_scope(root, mesh, inventory, override_mtl, None)
}

fn original_model_material_with_scope(
    root: &Path,
    mesh: &Path,
    inventory: &Inventory,
    override_mtl: Option<&str>,
    imported: Option<&ImportedTextureScope>,
) -> Result<Option<(String, PathBuf)>, String> {
    // Windows can spell an indexed temporary root with a \\?\\ prefix.
    // Compare canonical roots and meshes before producing virtual material paths.
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let mesh = fs::canonicalize(mesh).map_err(|e| e.to_string())?;
    let relative = mesh
        .strip_prefix(&root)
        .map_err(|_| "Model material lookup escaped original asset root")?;
    let virtual_mesh = relative.to_string_lossy().replace('\\', "/");
    let parent = virtual_mesh
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent);
    let stem = mesh
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or("Source mesh filename is not UTF-8")?;
    let mut paths = Vec::new();
    if override_mtl.is_none() {
        // Match the GPU viewer's MtlName precedence. These are original
        // compiled references, not speculative adjacent-owner fallbacks.
        let source = fs::symlink_metadata(&mesh).map_err(|e| e.to_string())?;
        if source.file_type().is_file() && source.len() <= 16 * 1024 * 1024 {
            let bytes = fs::read(&mesh).map_err(|e| e.to_string())?;
            let authored = compiled_material_names(&bytes)?;
            let mut hits = Vec::<(String, PathBuf)>::new();
            for reference in authored {
                let rooted = ["characters/", "objects/", "materials/", "levels/", "libs/"]
                    .iter()
                    .any(|prefix| reference.to_ascii_lowercase().starts_with(prefix));
                let mut candidates = Vec::new();
                if !rooted && !parent.is_empty() {
                    candidates.push(format!("{parent}/{reference}"));
                } else {
                    candidates.push(reference);
                }
                for candidate in candidates {
                    if let Some(found) = exact_authored_mtl(&root, inventory, &candidate, imported)?
                        && !hits.iter().any(|(_, path)| path == &found.1)
                    {
                        hits.push(found);
                    }
                }
            }
            if hits.len() > 1 {
                return Err(format!(
                    "Several conflicting original compiled MtlName references for {virtual_mesh}"
                ));
            }
            if let Some(hit) = hits.pop() {
                return Ok(Some(hit));
            }
        }
    }
    if let Some(authored) = override_mtl {
        let authored = authored.trim().replace('\\', "/");
        let rooted = ["characters/", "objects/", "materials/", "levels/", "libs/"]
            .iter()
            .any(|prefix| authored.to_ascii_lowercase().starts_with(prefix));
        if !rooted && !parent.is_empty() {
            paths.push(format!("{parent}/{authored}"));
        } else {
            paths.push(authored);
        }
    } else {
        paths.push(if parent.is_empty() {
            format!("{stem}.mtl")
        } else {
            format!("{parent}/{stem}.mtl")
        });
        use std::io::Read;
        let mut header = [0u8; 16];
        let historical = fs::File::open(&mesh)
            .and_then(|mut file| file.read_exact(&mut header))
            .is_ok()
            && &header[..8] == b"CryTek\0\0"
            && u32::from_le_bytes(header[12..16].try_into().unwrap_or_default()) == 0x745;
        if historical {
            let folder = parent.rsplit('/').next().unwrap_or("");
            let family = folder.split('_').next().unwrap_or("");
            let looks_owned = family.len() >= 4
                && stem
                    .to_ascii_lowercase()
                    .starts_with(&family.to_ascii_lowercase());
            if looks_owned {
                if let Some((variant, _)) = stem.split_once('_')
                    && variant.len() >= family.len()
                {
                    paths.push(format!("{parent}/{variant}.mtl"));
                }
                paths.push(format!("{parent}/{folder}.mtl"));
                paths.push(format!("{parent}/{family}.mtl"));
            }
        }
    }
    let mut tried = HashSet::new();
    for candidate in paths {
        if !tried.insert(candidate.to_ascii_lowercase()) {
            continue;
        }
        if let Some(found) = exact_authored_mtl(&root, inventory, &candidate, imported)? {
            return Ok(Some(found));
        }
    }
    if let Some(authored) = override_mtl {
        return Err(format!(
            "CDF attachment explicitly requires missing material {authored}; no other MTL can replace it"
        ));
    }
    Ok(None)
}

/// Resolve actual MTL texture roles, never the first same-named DDS found
/// somewhere else. Multiple submaterials need the original subset map and
/// are left untextured rather than painted with an incorrect atlas.
fn exact_single_slot_maps(
    root: &Path,
    mesh_path: &Path,
    inventory: &Inventory,
    override_mtl: Option<&str>,
) -> Result<AuthoredMaps, String> {
    exact_single_slot_maps_with_scope(root, mesh_path, inventory, override_mtl, None)
}

fn exact_single_slot_maps_with_scope(
    root: &Path,
    mesh_path: &Path,
    inventory: &Inventory,
    override_mtl: Option<&str>,
    imported: Option<&ImportedTextureScope>,
) -> Result<AuthoredMaps, String> {
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let Some((candidate, material)) =
        original_model_material_with_scope(&root, mesh_path, inventory, override_mtl, imported)?
    else {
        return Ok(AuthoredMaps::default());
    };
    let document = read_material(&material)
        .map_err(|e| format!("Invalid authored material {candidate}: {e}"))?;
    if document.slots.len() != 1 {
        return Ok(AuthoredMaps::default());
    }
    let mut maps = AuthoredMaps::default();
    let mut used_roles = HashSet::new();
    let leaf = &document.slots[0];
    let legacy_decal = leaf.shader.eq_ignore_ascii_case("Illum")
        && !leaf.alpha_glow
        && leaf.glow_amount.is_some_and(|amount| amount > 0.0)
        && !leaf
            .maps
            .iter()
            .any(|map| SOURCE_EMISSION_ROLES.contains(&map.role.as_str()));
    for item in &document.slots[0].maps {
        let target = match item.role.as_str() {
            "diffuse" => &mut maps.diffuse,
            "normal" | "normals" | "bumpmap" => &mut maps.normal,
            "emissive" | "emittance" | "emission" | "glow" | "emissivecolor" | "emissionmap"
            | "glowmap" | "selfillum" | "selfillumination" => &mut maps.emissive,
            "decal" if legacy_decal => &mut maps.emissive,
            "specular" | "specularcolor" => &mut maps.specular,
            "gloss" | "glossiness" | "smoothness" => &mut maps.gloss,
            _ => continue,
        };
        if !used_roles.insert(item.role.as_str()) || target.is_some() {
            return Err(format!(
                "Multiple original {} layers in {candidate}",
                item.role
            ));
        }
        *target = exact_original_texture(&root, inventory, &candidate, &item.path, imported)?;
    }
    Ok(maps)
}

/// Resolve a standalone rig's diffuse through its exact indexed author MTL.
/// A missing or ambiguous source is not replaced with a same-named texture
/// from a different character or archive.
pub(crate) fn standalone_authored_diffuse(
    root: &Path,
    mesh_path: &Path,
    inventory: &Inventory,
) -> Result<Option<PathBuf>, String> {
    exact_single_slot_maps(root, mesh_path, inventory, None).map(|maps| maps.diffuse)
}

/// Resolve authored CryEngine leaf maps for every original subset, including
/// empty NoDraw leaves, without shifting indexes or borrowing a neighbor DDS.
/// Distinct PNGs stay bound to the original material ID in Blender.
#[cfg(test)]
fn exact_multi_slot_materials(
    root: &Path,
    mesh_path: &Path,
    inventory: &Inventory,
    override_mtl: Option<&str>,
) -> Result<Option<Vec<OriginalMaterialPng>>, String> {
    exact_multi_slot_materials_with_scope(root, mesh_path, inventory, override_mtl, None)
}

fn exact_multi_slot_materials_with_scope(
    root: &Path,
    mesh_path: &Path,
    inventory: &Inventory,
    override_mtl: Option<&str>,
    imported: Option<&ImportedTextureScope>,
) -> Result<Option<Vec<OriginalMaterialPng>>, String> {
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let Some((virtual_mtl, material)) =
        original_model_material_with_scope(&root, mesh_path, inventory, override_mtl, imported)?
    else {
        return Ok(None);
    };
    let doc = read_material(&material)
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
        let legacy_decal = leaf.shader.eq_ignore_ascii_case("Illum")
            && !leaf.alpha_glow
            && leaf.glow_amount.is_some_and(|amount| amount > 0.0)
            && !leaf
                .maps
                .iter()
                .any(|map| SOURCE_EMISSION_ROLES.contains(&map.role.as_str()));
        for map in &leaf.maps {
            let role = match map.role.as_str() {
                "diffuse" => 0,
                "normal" | "normals" | "bumpmap" => 1,
                "emittance" | "emissive" | "emission" | "glow" | "emissivecolor"
                | "emissionmap" | "glowmap" | "selfillum" | "selfillumination" => 2,
                "decal" if legacy_decal => 2,
                "specular" | "specularcolor" => 3,
                "gloss" | "glossiness" | "smoothness" => 4,
                _ => continue,
            };
            let Some(file) =
                exact_original_texture(&root, inventory, &virtual_mtl, &map.path, imported)?
            else {
                continue;
            };
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
                authored_roughness_png(&file, imported)?
            } else {
                authored_png(&file, role == 1, imported)?
            });
        }
        slots.push(slot);
    }
    Ok(Some(slots))
}

/// Compose original MTL BLENDLAYER into the exact subset's diffuse PNG.
/// Authoritative source paths, rather than filename-scanned damage assets,
/// select the damaged base and grayscale blend mask. Source alpha is not
/// used to remove geometry; original UVs/material IDs remain unchanged.
fn apply_export_damage_layer(
    root: &Path,
    original: &Path,
    inventory: &Inventory,
    material_hint: Option<&str>,
    imported: Option<&ImportedTextureScope>,
    slots: &mut [OriginalMaterialPng],
    percent: u8,
) -> Result<usize, String> {
    if percent == 0 {
        return Ok(0);
    }
    let Some((virtual_mtl, material)) =
        original_model_material_with_scope(root, original, inventory, material_hint, imported)?
    else {
        return Ok(0);
    };
    let doc = read_material(&material).map_err(|e| e.to_string())?;
    if doc.slots.len() != slots.len() {
        return Err("Original damage material does not match the source subset layout".into());
    }
    let mut applied = 0;
    for (leaf, target) in doc.slots.iter().zip(slots.iter_mut()) {
        let Some(base_png) = target.diffuse.as_ref() else {
            continue;
        };
        if target.no_draw {
            continue;
        }
        let Some((alternate_ref, blend_ref)) = crate::materials::authored_damage_layer(leaf) else {
            continue;
        };
        let alternate_path =
            exact_original_texture(root, inventory, &virtual_mtl, alternate_ref, imported)?
                .ok_or_else(|| format!("Missing authored alternate damage map {alternate_ref}"))?;
        let blend_path =
            exact_original_texture(root, inventory, &virtual_mtl, blend_ref, imported)?
                .ok_or_else(|| format!("Missing authored damage blend mask {blend_ref}"))?;
        let base = image::load_from_memory(base_png)
            .map_err(|e| format!("Invalid original subset diffuse PNG: {e}"))?
            .to_rgba8();
        let (_, alternate) = decode_original_mip(&alternate_path, imported)?;
        let (_, mask) = decode_original_mip(&blend_path, imported)?;
        let mixed = crate::materials::blend_damage_rgba(&base, &alternate, &mask, percent)?;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                mixed.as_raw(),
                mixed.width(),
                mixed.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| format!("Cannot encode blended damage diffuse: {e}"))?;
        target.diffuse = Some(png);
        applied += 1;
    }
    Ok(applied)
}

/// glTF metallicRoughnessTexture uses roughness in green and metallic in
/// blue. An explicitly authored gloss map is *inverse* roughness; never
/// derive it from arbitrary normal, specular or diffuse alpha.
fn authored_roughness_png(
    path: &Path,
    imported: Option<&ImportedTextureScope>,
) -> Result<Vec<u8>, String> {
    let (_info, mut rgba) = decode_original_mip(path, imported)?;
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

fn authored_png(
    path: &Path,
    normal_map: bool,
    imported: Option<&ImportedTextureScope>,
) -> Result<Vec<u8>, String> {
    let (info, mut rgba) = decode_original_mip(path, imported)?;
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

/// Preserve the local CryEngine material-ID space of *each* original
/// attachment when exporting a flattened, unrigged CDF. A slot ID of zero on
/// Emet's face is not the same material as slot zero on his weapon. Each part
/// therefore owns a non-overlapping material range in the combined glTF, while
/// its original ID and authored MTL leaf are retained in the material name.
/// The input report is generated from the exact same verified source geometry,
/// so the recorded triangle spans must cover the complete source index buffer.
pub(crate) fn static_cdf_material_glb(
    mesh: &crate::model::RenderMesh,
    report: &crate::character_assembly::AssemblyReport,
    inventory: &Inventory,
    embed_textures: bool,
) -> Result<Vec<u8>, String> {
    let root = fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
    let scope = texture_scope(inventory);
    let mut subsets = Vec::<(u32, u32, u32)>::new();
    let mut slots = Vec::<OriginalMaterialPng>::new();
    let mut cursor = 0usize;
    let mut expected_index = 0u32;
    if report.geometry_parts.is_empty() || report.source_parts.is_empty() {
        return Err("CDF has no original attachment material assignments".into());
    }
    for (part_index, part) in report.geometry_parts.iter().enumerate() {
        let first = cursor;
        while cursor < report.source_parts.len()
            && report.source_parts[cursor].name == part.name
        {
            cursor += 1;
        }
        let authored = &report.source_parts[first..cursor];
        if authored.is_empty() {
            return Err(format!(
                "CDF part {} has no original mesh subset material IDs",
                part.name
            ));
        }
        let override_material = authored[0].material_hint.as_deref();
        if authored.iter().any(|entry| entry.material_hint.as_deref() != override_material) {
            return Err(format!(
                "CDF part {} has conflicting original material overrides",
                part.name
            ));
        }
        let original = exact_multi_slot_materials_with_scope(
            &root,
            &part.source,
            inventory,
            override_material,
            scope.as_ref(),
        )?;
        let raw = fs::read(&part.source).map_err(|e| e.to_string())?;
        let original_table = has_original_mesh_subset_table(&raw)
            .map_err(|e| format!("{}: {e}", part.name))?;
        if !original_table && original.as_ref().is_some_and(|list| list.len() > 1) {
            return Err(format!(
                "CDF part {} has a multi-leaf original MTL without a source subset table",
                part.name
            ));
        }
        let max_id = authored
            .iter()
            .map(|entry| entry.material_slot)
            .max()
            .ok_or("CDF source part lost its material slots")?;
        if max_id >= 128
            || authored.iter().any(|entry| entry.material_slot == 255)
        {
            return Err(format!("CDF part {} has an unresolved material ID", part.name));
        }
        let mut leaves = if let Some(leaves) = original {
            if max_id as usize >= leaves.len() {
                return Err(format!(
                    "CDF part {} uses source material ID {} outside its original MTL leaves",
                    part.name, max_id
                ));
            }
            leaves
        } else {
            // Preserve known numeric source IDs even if the original MTL is
            // not indexed. No color or normal-map texture is fabricated.
            (0..=max_id)
                .map(|id| OriginalMaterialPng {
                    name: format!("Unresolved MTL leaf {id}"),
                    ..Default::default()
                })
                .collect::<Vec<_>>()
        };
        if slots.len().checked_add(leaves.len()).is_none_or(|n| n > 128) {
            return Err("CDF has more than 128 source material leaves".into());
        }
        let offset = u32::try_from(slots.len())
            .map_err(|_| "CDF material offset overflow")?;
        for (id, leaf) in leaves.iter_mut().enumerate() {
            let suffix = leaf.name.chars().take(90).collect::<String>();
            leaf.name = format!(
                "CDF part {part_index:03} {} | original ID {id:03} | {suffix}",
                part.name
            );
            if !embed_textures {
                // Embedded Textures OFF disables diffuse/specular/emissive
                // embedding, NEVER original material IDs or normal maps.
                leaf.diffuse = None;
                leaf.emissive = None;
                leaf.specular = None;
                leaf.roughness = None;
            }
        }
        for subset in authored {
            if subset.first_index != expected_index
                || subset.index_count == 0
                || !subset.index_count.is_multiple_of(3)
            {
                return Err(format!(
                    "CDF part {} has gaps or invalid original subset triangles",
                    part.name
                ));
            }
            let end = expected_index
                .checked_add(subset.index_count)
                .ok_or("CDF mesh index span overflow")?;
            if end as usize > mesh.indices.len() {
                return Err("CDF source subset exceeds combined model".into());
            }
            let mapped = offset
                .checked_add(subset.material_slot)
                .ok_or("CDF source material ID offset overflow")?;
            subsets.push((expected_index, subset.index_count, mapped));
            expected_index = end;
        }
        slots.extend(leaves);
    }
    if cursor != report.source_parts.len()
        || expected_index as usize != mesh.indices.len()
    {
        return Err("CDF original material IDs do not cover all source faces".into());
    }
    let base = crate::glb::geometry_glb_bytes_for_fbx(mesh)
        .map_err(|e| format!("CDF source GLB: {e}"))?;
    embed_original_material_slots_glb(&base, &subsets, &slots)
        .map_err(|e| format!("CDF original material/normal mapping: {e}"))
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
        source,
        inventory,
        blender,
        destination,
        caf,
        equipment,
        true,
    )
}

/// A rigged FBX with "Embedded textures" selected must include every
/// visible, original CryEngine material's diffuse. An FBX can carry valid
/// geometry, UV0 and a single material while containing *zero* image/video
/// objects (observed on original 2012 Scorpid 3 exports).
fn verify_embedded_source_diffuse(
    part: &str,
    subsets: &[(u32, u32, u32)],
    slots: &[OriginalMaterialPng],
) -> Result<(), String> {
    if subsets.is_empty() {
        return Err(format!("{part}: no original material subsets"));
    }
    for &(first, count, index) in subsets {
        if count == 0 {
            return Err(format!("{part}: empty original mesh subset at {first}"));
        }
        let material = slots
            .get(index as usize)
            .ok_or_else(|| format!("{part}: original material slot {index} is missing"))?;
        if !material.no_draw && material.diffuse.is_none() {
            return Err(format!(
                "{part}: original material slot {index} ({}) has no verified diffuse DDS. Embedded-texture export would produce an untextured 2012 mesh. Index its matching source DDS and retry.",
                material.name
            ));
        }
    }
    Ok(())
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
    export_unified_cdf_fbx_options_with_glow(
        source,
        inventory,
        blender,
        destination,
        caf,
        equipment,
        CdfExportAppearance {
            embed_textures,
            full_armor_glow: false,
            damage_blend_percent: 0,
        },
    )
}

/// Model Options affecting the appearance of a single CDF FBX.
#[derive(Clone, Copy, Debug, Default)]
pub struct CdfExportAppearance {
    pub embed_textures: bool,
    pub full_armor_glow: bool,
    /// Requested preview percentage for the authored Custom/Opacity damage blend.
    pub damage_blend_percent: u8,
}

/// Export selected CDF parts/equipment, including the optional source-exact
/// full-armor glow emission used in Model Options.
pub fn export_unified_cdf_fbx_options_with_glow(
    source: &Path,
    inventory: &Inventory,
    blender: &Path,
    destination: &Path,
    caf: Option<&Path>,
    equipment: &EquipmentOverrides,
    appearance: CdfExportAppearance,
) -> Result<UnifiedCdfFbxReport, String> {
    let CdfExportAppearance {
        embed_textures,
        full_armor_glow,
        damage_blend_percent,
    } = appearance;
    let root = fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
    let imported = texture_scope(inventory);
    if damage_blend_percent > 100 {
        return Err("Damage blend intensity must be between 0 and 100 percent".into());
    }
    if damage_blend_percent > 0 && !embed_textures {
        return Err("Damage blend requires embedded textures in the rigged CDF FBX".into());
    }
    if full_armor_glow && !embed_textures {
        return Err("Full-armor glow requires embedded textures in a rigged FBX".into());
    }
    let glow_png = if full_armor_glow {
        let path = monster_glow_path(source, inventory)?
            .ok_or("No exact full-armor glow DDS belongs to this monster and build")?;
        Some(
            authored_png(&path, false, imported.as_ref())
                .map_err(|e| format!("Full-armor glow DDS: {e}"))?,
        )
    } else {
        None
    };
    let relative_cdf = fs::canonicalize(source)
        .map_err(|e| e.to_string())?
        .strip_prefix(&root)
        .map_err(|_| "CDF outside indexed source root")?
        .to_string_lossy()
        .replace('\\', "/");
    let monster_dir = relative_cdf
        .rsplit_once('/')
        .map(|(dir, _)| format!("{dir}/").to_ascii_lowercase());
    let mut applied_glow = false;
    let mut applied_damage = 0usize;
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
    let cdf_skeleton_bytes = fs::read(&skeleton).map_err(|e| e.to_string())?;
    // Rocky's historical original master has a baseroot that is not in
    // its two 47-bone skinned parts. The legacy converter correctly
    // refuses to retarget incompatible weighted bind poses. If EVERY
    // selected historical part has the same verified local bind pose,
    // rig all parts against that one original part-local skeleton instead
    // of allowing one part to use the 48-bone master and another 47.
    // No inference, fabricated joints or altered game file is involved.
    let local_skeleton = if cdf_skeleton_bytes.starts_with(b"CryTek\0\0") {
        let mut candidate: Option<Vec<u8>> = None;
        let mut identical = true;
        for (_, path, _) in &parts {
            let raw = fs::read(path).map_err(|e| e.to_string())?;
            if !raw.starts_with(b"CryTek\0\0") {
                identical = false;
                break;
            }
            if let Some(first) = &candidate {
                if !crate::legacy_rig::identical_legacy_bind_skeletons(first, &raw)
                    .map_err(|e| format!("Verifying 2012 CDF part bind poses: {e}"))?
                {
                    identical = false;
                    break;
                }
            } else {
                candidate = Some(raw);
            }
        }
        identical.then_some(candidate).flatten()
    } else {
        None
    };
    let skeleton_bytes = local_skeleton.as_deref().unwrap_or(&cdf_skeleton_bytes);
    // Real original 2012 Rocky attachments do not all share the same rest
    // skeleton: two parts use 47 joints while the left arm has 48. Keep the
    // per-part, verified local bind skeleton when shared retargeting fails.
    // Blender will combine only identical rigs and retain distinct verified
    // historical armatures within the same FBX. No joint is discarded, added
    // or reassigned to make counts match.
    let allow_distinct_historical_rigs = cdf_skeleton_bytes.starts_with(b"CryTek\0\0")
        && parts.iter().all(|(_, path, _)| {
            fs::File::open(path)
                .and_then(|mut file| {
                    use std::io::Read;
                    let mut magic = [0u8; 8];
                    file.read_exact(&mut magic)?;
                    Ok(&magic == b"CryTek\0\0")
                })
                .unwrap_or(false)
        });
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
    for (index, (name, original, material_hint)) in parts.iter().enumerate() {
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
            (Some(clip), 0) => rigged_glb_bytes_with_caf(&mesh, &raw, skeleton_bytes, clip),
            _ => rigged_glb_bytes(&mesh, &raw, skeleton_bytes),
        }
        .map_err(|e| format!("CDF part {name} cannot be rigged: {e}"))?;
        if let Some(previous) = bone_count
            && previous != verified.bone_count
            && !allow_distinct_historical_rigs
        {
            return Err(format!(
                "CDF part {name} has {} verified bones, but the preceding parts have {previous}; original rest skeletons are incompatible",
                verified.bone_count
            ));
        }
        // For legitimate historical per-part rigs this represents the largest
        // individual bone count, NOT an invented joint count for a new skeleton.
        bone_count = Some(bone_count.unwrap_or(0).max(verified.bone_count));
        vertices = vertices
            .checked_add(verified.weighted_vertices)
            .ok_or("Vertex count overflow")?;
        let source_subsets = original_mesh_subsets(&raw, mesh.indices.len())
            .map_err(|e| format!("CDF part {name} source subset mapping: {e}"))?;
        let original_materials = exact_multi_slot_materials_with_scope(
            &root,
            original,
            inventory,
            material_hint.as_deref(),
            imported.as_ref(),
        )?;
        // Keep each original source subset ID and triangle count as a contract
        // for Blender: material separation and FBX reimport must not remap faces
        // to adjacent MTL leaves (notably empty NoDraw equipment slots).
        let original_table = has_original_mesh_subset_table(&raw)
            .map_err(|e| format!("CDF part {name} source material ID table: {e}"))?;
        if !original_table
            && original_materials
                .as_ref()
                .is_some_and(|slots| slots.len() > 1)
        {
            return Err(format!(
                "CDF part {name}: source MTL has multiple leaves but the mesh has no original subset table. Material IDs cannot safely be resolved."
            ));
        }
        let mut material_face_counts = std::collections::BTreeMap::<u32, u64>::new();
        for &(_, count, source_id) in &source_subsets {
            if source_id == 255 {
                return Err(format!(
                    "CDF part {name} has unresolved source material ID 255"
                ));
            }
            if let Some(slots) = &original_materials
                && source_id as usize >= slots.len()
            {
                return Err(format!(
                    "CDF part {name}: subset material ID {source_id} exceeds original MTL leaf count {}",
                    slots.len()
                ));
            }
            *material_face_counts.entry(source_id).or_insert(0) += u64::from(count / 3);
        }
        let original_material_ids = material_face_counts
            .iter()
            .map(|(&id, &triangles)| {
                let source_leaf = original_materials
                    .as_ref()
                    .and_then(|slots| slots.get(id as usize));
                let leaf = source_leaf
                    .map(|slot| slot.name.as_str())
                    .unwrap_or("Source material (no MTL leaf)");
                let no_draw = source_leaf.is_some_and(|slot| slot.no_draw);
                json!({"id":id, "triangles":triangles, "name":leaf,
                       "no_draw":no_draw})
            })
            .collect::<Vec<_>>();
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
            let mut slots = original_materials.ok_or_else(|| {
                format!("CDF part {name} has several subsets but no original source MTL")
            })?;
            if source_subsets.iter().any(|(_, _, index)| *index == 255) {
                return Err(format!("CDF part {name} has an unresolved material subset"));
            }
            if embed_textures {
                verify_embedded_source_diffuse(name, &source_subsets, &slots)?;
            }
            if slots.iter().any(|slot| slot.diffuse.is_some()) {
                diffuse_textured_parts += 1;
            }
            if damage_blend_percent > 0
                && monster_dir.as_deref().is_some_and(|owner| {
                    original.strip_prefix(&root).is_ok_and(|relative| {
                        relative
                            .to_string_lossy()
                            .replace('\\', "/")
                            .to_ascii_lowercase()
                            .starts_with(owner)
                    })
                })
            {
                applied_damage += apply_export_damage_layer(
                    &root,
                    original,
                    inventory,
                    material_hint.as_deref(),
                    imported.as_ref(),
                    &mut slots,
                    damage_blend_percent,
                )?;
            }
            // Only original parts of THIS monster get the glow overlay;
            // equipment bindings in other folders remain untouched.
            if let (Some(glow), Some(owner)) = (&glow_png, &monster_dir)
                && original.strip_prefix(&root).is_ok_and(|relative| {
                    relative
                        .to_string_lossy()
                        .replace('\\', "/")
                        .to_ascii_lowercase()
                        .starts_with(owner)
                })
            {
                for slot in slots
                    .iter_mut()
                    .filter(|slot| slot.diffuse.is_some() && !slot.no_draw)
                {
                    slot.emissive = Some(glow.clone());
                    applied_glow = true;
                }
            }
            glb = embed_original_material_slots_glb(&glb, &source_subsets, &slots)
                .map_err(|e| format!("CDF part {name} original multi-material: {e}"))?;
        } else {
            let maps = exact_single_slot_maps_with_scope(
                &root,
                original,
                inventory,
                material_hint.as_deref(),
                imported.as_ref(),
            )?;
            let mut diffuse = maps
                .diffuse
                .as_deref()
                .map(|f| authored_png(f, false, imported.as_ref()))
                .transpose()?;
            if damage_blend_percent > 0
                && monster_dir.as_deref().is_some_and(|owner| {
                    original.strip_prefix(&root).is_ok_and(|relative| {
                        relative
                            .to_string_lossy()
                            .replace('\\', "/")
                            .to_ascii_lowercase()
                            .starts_with(owner)
                    })
                })
            {
                let mut slots = vec![OriginalMaterialPng {
                    name: name.clone(),
                    diffuse: diffuse.take(),
                    ..Default::default()
                }];
                applied_damage += apply_export_damage_layer(
                    &root,
                    original,
                    inventory,
                    material_hint.as_deref(),
                    imported.as_ref(),
                    &mut slots,
                    damage_blend_percent,
                )?;
                diffuse = slots.pop().and_then(|slot| slot.diffuse);
            }
            let normal = maps
                .normal
                .as_deref()
                .map(|f| authored_png(f, true, imported.as_ref()))
                .transpose()?;
            let mut emissive = maps
                .emissive
                .as_deref()
                .map(|f| authored_png(f, false, imported.as_ref()))
                .transpose()?;
            if let (Some(glow), Some(owner)) = (&glow_png, &monster_dir)
                && diffuse.is_some()
                && original.strip_prefix(&root).is_ok_and(|relative| {
                    relative
                        .to_string_lossy()
                        .replace('\\', "/")
                        .to_ascii_lowercase()
                        .starts_with(owner)
                })
            {
                emissive = Some(glow.clone());
                applied_glow = true;
            }
            let specular = maps
                .specular
                .as_deref()
                .map(|f| authored_png(f, false, imported.as_ref()))
                .transpose()?;
            let roughness = maps
                .gloss
                .as_deref()
                .map(|f| authored_roughness_png(f, imported.as_ref()))
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
            if embed_textures && diffuse.is_none() {
                return Err(format!(
                    "CDF part {name}: no original diffuse DDS resolved from its authored 2012 material. Embedded-texture export would lose the part's color map; import the matching DDS archive and retry."
                ));
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
        entries.push(json!({
            "name":name,
            "source":staged,
            "material_ids":original_material_ids
        }));
        labels.push(name.clone());
    }
    if damage_blend_percent > 0 && applied_damage == 0 {
        return Err("No verified authored Custom damage diffuse and Opacity blend mask were found in the selected monster CDF".into());
    }
    if full_armor_glow && !applied_glow {
        return Err(
            "Full-armor glow had no verified diffuse mesh section in the selected monster CDF"
                .into(),
        );
    }
    if embed_textures && diffuse_textured_parts == 0 {
        return Err(
            "Embedded textures requested, but no original character diffuse DDS was resolved. Import the matching source MTL and highest-resolution DDS into the indexed character assets; the FBX cannot be honestly labelled textured.".into(),
        );
    }
    let manifest = work.path().join("parts.json");
    fs::write(
        &manifest,
        serde_json::to_vec(&json!({
            "parts":entries, "include_animation":motion.is_some(),
            "embed_textures":embed_textures,
            "allow_distinct_historical_armatures":allow_distinct_historical_rigs
        }))
        .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let script = work.path().join("unify.py");
    fs::write(&script, include_str!("blender_character_fbx.py")).map_err(|e| e.to_string())?;
    fs::write(
        work.path().join("blender_uv0.py"),
        include_str!("blender_uv0.py"),
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        work.path().join("blender_fbx_safety.py"),
        include_str!("blender_fbx_safety.py"),
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        work.path().join("blender_material_ids.py"),
        include_str!("blender_material_ids.py"),
    )
    .map_err(|e| e.to_string())?;
    let temp_fbx = work.path().join(
        destination
            .file_name()
            .ok_or("FBX output file name missing")?,
    );
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
    // Blender saves this only after importing the exported FBX back and
    // revalidating every original armature, mesh binding, weight and UV.
    // Never publish a success without this round-trip confirmation.
    let verification: serde_json::Value = serde_json::from_slice(
        &fs::read(work.path().join("verified-export.json"))
            .map_err(|e| format!("Missing Blender original-rig validation: {e}"))?,
    )
    .map_err(|e| format!("Invalid Blender original-rig validation: {e}"))?;
    let rig_count = verification["armatures"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or("Missing verified FBX armature count")?;
    let verified_bones = verification["bones"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or("Missing verified FBX bone count")?;
    let verified_parts = verification["parts"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or("Missing verified FBX part count")?;
    if rig_count == 0
        || rig_count > parts.len()
        || (!allow_distinct_historical_rigs && rig_count != 1)
        || verified_bones < bone_count.unwrap_or(0)
        || verified_bones > parts.len() * 1024
        || verified_parts != parts.len()
    {
        return Err("Blender original-rig verification counts disagree with source parts".into());
    }
    // Shares the same byte-for-byte, non-overwriting FBX publish path as
    // single-model exports. The old flow consumed the first 23 bytes while
    // verifying the signature and accidentally discarded them on copy.
    publish_verified_binary_fbx_with_normals(&temp_fbx, destination, !embed_textures)
        .map_err(|e| e.to_string())?;
    Ok(UnifiedCdfFbxReport {
        original_skeleton: skeleton_ref,
        selected_skin_set: skin_set,
        parts: labels,
        bones: verified_bones,
        armatures: rig_count,
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
        let diffuse = exact_single_slot_maps(root, &compiled, &inventory, None)
            .unwrap()
            .diffuse
            .unwrap();
        assert!(diffuse.ends_with("hunter.dds.0"));
        assert_eq!(
            standalone_authored_diffuse(root, &compiled, &inventory).unwrap(),
            Some(diffuse)
        );
        assert_eq!(fs::read(original).unwrap(), b"original binding");
    }

    /// Minimal validated CrChF MtlName record; uses the same compiled
    /// material table structure recognized by the GPU viewer.
    fn source_mesh_with_mtl_names(names: &[&str]) -> Vec<u8> {
        let mut bytes = vec![0u8; 32 + names.len() * 16];
        bytes[..5].copy_from_slice(b"CrChF");
        bytes[8..12].copy_from_slice(&(names.len() as u32).to_le_bytes());
        bytes[12..16].copy_from_slice(&32u32.to_le_bytes());
        for (i, name) in names.iter().enumerate() {
            assert!(name.len() < 127);
            let at = 32 + i * 16;
            let offset = bytes.len();
            bytes[at..at + 4].copy_from_slice(&0x08021014u32.to_le_bytes());
            bytes[at + 8..at + 12].copy_from_slice(&132u32.to_le_bytes());
            bytes[at + 12..at + 16].copy_from_slice(&(offset as u32).to_le_bytes());
            let mut name_record = vec![0u8; 132];
            name_record[..name.len()].copy_from_slice(name.as_bytes());
            bytes.extend_from_slice(&name_record);
        }
        bytes
    }

    #[test]
    fn emet_offhand_part_uses_authored_mtl_name_before_same_stem() {
        let source = tempfile::tempdir().unwrap();
        let folder = source.path().join("characters/hunters/merc_emet");
        fs::create_dir_all(&folder).unwrap();
        // Emet's CDF attachment is named "oh", which is *not* the material
        // filename. The original cooked mesh points at emet_3o.mtl.
        let part = folder.join("oh.skinm");
        fs::write(&part, source_mesh_with_mtl_names(&["emet_3o"])).unwrap();
        fs::write(
            folder.join("emet_3o.mtl"),
            br#"<Material Name="Emet"><SubMaterials>
               <Material Name="Body" Shader="Illum"/>
               <Material Name="Equipment" Shader="Illum"/>
               </SubMaterials></Material>"#,
        )
        .unwrap();
        // This same-stem file is deliberately not the compiled source
        // material; finding it first would silently assign wrong subset IDs.
        fs::write(folder.join("oh.mtl"), br#"<Material Name="Wrong"/>"#).unwrap();
        let inventory = crate::scan_directory(source.path()).unwrap();
        let (virtual_name, file) = original_model_material(source.path(), &part, &inventory, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            virtual_name.to_ascii_lowercase(),
            "characters/hunters/merc_emet/emet_3o.mtl"
        );
        assert_eq!(file, fs::canonicalize(folder.join("emet_3o.mtl")).unwrap());
        let slots =
            exact_multi_slot_materials_with_scope(source.path(), &part, &inventory, None, None)
                .unwrap()
                .unwrap();
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].name, "Body");
        assert_eq!(slots[1].name, "Equipment");
        // Explicit per-attachment CDF material always wins over MtlName.
        let (explicit, _) =
            original_model_material(source.path(), &part, &inventory, Some("oh.mtl"))
                .unwrap()
                .unwrap();
        assert_eq!(
            explicit.to_ascii_lowercase(),
            "characters/hunters/merc_emet/oh.mtl"
        );
    }

    #[test]
    fn emet_multi_slot_mtl_can_resolve_from_verified_same_build_pak() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("characters/hunters/merc_emet");
        fs::create_dir_all(&folder).unwrap();
        let part = folder.join("oh.skinm");
        fs::write(&part, source_mesh_with_mtl_names(&["emet_3o"])).unwrap();
        let inventory = crate::scan_directory(root.path()).unwrap();
        let library = tempfile::tempdir().unwrap();
        let archive_id = "11111111111111111111";
        let archive = library
            .path()
            .join(format!("ui_characters.pak__{archive_id}"));
        let virtual_mtl = "characters/hunters/merc_emet/emet_3o.mtl";
        let mtl = archive.join("files").join(virtual_mtl);
        fs::create_dir_all(mtl.parent().unwrap()).unwrap();
        let xml = br#"<Material Name="Emet"><SubMaterials>
            <Material Name="slot0" Shader="Illum"/>
            <Material Name="slot1" Shader="Illum"/>
            </SubMaterials></Material>"#;
        fs::write(&mtl, xml).unwrap();
        fs::write(
            archive.join("asset-library.json"),
            serde_json::to_vec(&serde_json::json!({
                "version":1,"name":"ui_characters.pak",
                "source":"C:/Game/ui_characters.pak",
                "entries":[{"path":virtual_mtl,"bytes":xml.len()}]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            library.path().join("build-collections.json"),
            serde_json::to_vec(&serde_json::json!({
                "version":1,
                "builds":{"stage-two":{
                    "label":"Stage 2",
                    "members":{"11111111111111111111":{}}
                }}
            }))
            .unwrap(),
        )
        .unwrap();
        let scope = ImportedTextureScope {
            root: library.path().to_path_buf(),
            build: Some("Stage 2".into()),
        };
        let slots = exact_multi_slot_materials_with_scope(
            root.path(),
            &part,
            &inventory,
            None,
            Some(&scope),
        )
        .unwrap()
        .unwrap();
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].name, "slot0");
        assert_eq!(slots[1].name, "slot1");
        let wrong_build = ImportedTextureScope {
            root: library.path().to_path_buf(),
            build: Some("Legacy 2012".into()),
        };
        assert!(
            original_model_material_with_scope(
                root.path(),
                &part,
                &inventory,
                None,
                Some(&wrong_build),
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn compiled_material_name_rejects_conflicts_without_borrowing_another_actor() {
        let source = tempfile::tempdir().unwrap();
        let folder = source.path().join("characters/hunters/merc_emet");
        let other = source.path().join("characters/hunters/merc_other");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir_all(&other).unwrap();
        let part = folder.join("oh.skinm");
        fs::write(&part, source_mesh_with_mtl_names(&["emet_3o"])).unwrap();
        fs::write(other.join("emet_3o.mtl"), b"<Material/>").unwrap();
        let inventory = crate::scan_directory(source.path()).unwrap();
        assert!(
            original_model_material(source.path(), &part, &inventory, None)
                .unwrap()
                .is_none()
        );
        fs::write(folder.join("emet_3o.mtl"), b"<Material/>").unwrap();
        fs::write(folder.join("emet_alt.mtl"), b"<Material/>").unwrap();
        fs::write(&part, source_mesh_with_mtl_names(&["emet_3o", "emet_alt"])).unwrap();
        let inventory = crate::scan_directory(source.path()).unwrap();
        assert!(
            original_model_material(source.path(), &part, &inventory, None)
                .unwrap_err()
                .contains("conflicting")
        );
        assert!(
            compiled_material_names(&source_mesh_with_mtl_names(&[
                "characters\\hunters\\merc_emet\\emet_3o"
            ]))
            .unwrap()[0]
                .ends_with("emet_3o.mtl")
        );
        assert!(
            compiled_material_names(b"invalid legacy stream")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn fbx_source_material_texture_matches_viewer_relative_highest_mip() {
        let root = tempfile::tempdir().unwrap();
        let model = root.path().join("characters/monsters/scorpid_2");
        fs::create_dir_all(model.join("textures")).unwrap();
        // Model needs a real source path for the canonical ownership check.
        fs::write(model.join("scorpid3.chr"), b"source render geometry").unwrap();
        fs::write(
            model.join("scorpid3.mtl"),
            br#"<Material Name="Scorpid"><Textures><Texture Map="Diffuse" File="body.tif"/></Textures></Material>"#,
        )
        .unwrap();
        let mut dds = vec![0u8; 136];
        dds[..4].copy_from_slice(b"DDS ");
        dds[4..8].copy_from_slice(&124u32.to_le_bytes());
        dds[8..12].copy_from_slice(&0x1007u32.to_le_bytes());
        dds[12..16].copy_from_slice(&4u32.to_le_bytes());
        dds[16..20].copy_from_slice(&4u32.to_le_bytes());
        dds[76..80].copy_from_slice(&32u32.to_le_bytes());
        dds[80..84].copy_from_slice(&4u32.to_le_bytes());
        dds[84..88].copy_from_slice(b"DXT1");
        dds[128..136].copy_from_slice(&[0, 248, 0, 0, 0, 0, 0, 0]);
        fs::write(model.join("textures/body.dds.0"), &dds).unwrap();
        fs::write(model.join("textures/body.dds"), b"lower-priority").unwrap();
        let inventory = crate::scan_directory(root.path()).unwrap();
        let single =
            exact_single_slot_maps(root.path(), &model.join("scorpid3.chr"), &inventory, None)
                .unwrap();
        assert_eq!(
            fs::canonicalize(single.diffuse.unwrap()).unwrap(),
            fs::canonicalize(model.join("textures/body.dds.0")).unwrap()
        );
        let multiple =
            exact_multi_slot_materials(root.path(), &model.join("scorpid3.chr"), &inventory, None)
                .unwrap()
                .unwrap();
        assert_eq!(multiple.len(), 1);
        assert!(
            multiple[0]
                .diffuse
                .as_ref()
                .is_some_and(|png| png.starts_with(&[137, 80, 78, 71]))
        );
    }

    #[test]
    fn legacy_cdf_variant_picks_only_original_owned_material_or_explicit_override() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let parent = root.join("objects/characters/monsters/scorpid_2");
        fs::create_dir_all(&parent).unwrap();
        let mut body = vec![0u8; 20];
        body[..8].copy_from_slice(b"CryTek\0\0");
        body[8..12].copy_from_slice(&0xFFFF0000u32.to_le_bytes());
        body[12..16].copy_from_slice(&0x745u32.to_le_bytes());
        let part = parent.join("scorpid3_rig.chr");
        fs::write(&part, &body).unwrap();
        fs::write(
            parent.join("scorpid3.mtl"),
            br#"<Material Name="Correct"/>"#,
        )
        .unwrap();
        fs::write(parent.join("scorpid2.mtl"), br#"<Material Name="Other"/>"#).unwrap();
        let inventory = crate::scan_directory(root).unwrap();
        let (relative, _) = original_model_material(root, &part, &inventory, None)
            .unwrap()
            .unwrap();
        assert!(
            relative.eq_ignore_ascii_case("objects/characters/monsters/scorpid_2/scorpid3.mtl")
        );
        let (explicit, _) = original_model_material(
            root,
            &part,
            &inventory,
            Some("objects/characters/monsters/scorpid_2/scorpid2.mtl"),
        )
        .unwrap()
        .unwrap();
        assert!(explicit.ends_with("scorpid2.mtl"));
        assert!(
            original_model_material(
                root,
                &part,
                &inventory,
                Some("objects/characters/monsters/scorpid_2/missing.mtl"),
            )
            .is_err()
        );
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
            exact_single_slot_maps(root, &mesh, &inventory, None)
                .unwrap()
                .diffuse
                .is_none()
        );
    }

    #[test]
    fn monster_glow_uses_selected_variant_and_original_character_folder_only() {
        let dir = tempfile::tempdir().unwrap();
        let owner = dir.path().join("objects/characters/monsters/goliath");
        let neighbor = dir.path().join("objects/characters/monsters/scorpid_2");
        fs::create_dir_all(&owner).unwrap();
        fs::create_dir_all(&neighbor).unwrap();
        fs::write(owner.join("goliath3.cdf"), b"cdf selection").unwrap();
        fs::write(owner.join("goliath2_glow_diff.dds.0"), b"other stage").unwrap();
        fs::write(neighbor.join("goliath3_glow_diff.dds.0"), b"other actor").unwrap();
        let inventory = crate::scan_directory(dir.path()).unwrap();
        assert!(
            monster_glow_path(&owner.join("goliath3.cdf"), &inventory)
                .unwrap()
                .is_none(),
            "Another stage/monster must never supply a glow texture"
        );
        let correct = owner.join("goliath3_glow_diff.dds.0");
        fs::write(&correct, b"exact stage glow").unwrap();
        let inventory = crate::scan_directory(dir.path()).unwrap();
        assert_eq!(
            fs::canonicalize(
                monster_glow_path(&owner.join("goliath3.cdf"), &inventory)
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            fs::canonicalize(correct).unwrap()
        );
        fs::create_dir_all(dir.path().join("characters/hunters")).unwrap();
        let hunter = dir.path().join("characters/hunters/hunter.cdf");
        fs::write(&hunter, b"hunter CDF").unwrap();
        let inventory = crate::scan_directory(dir.path()).unwrap();
        assert!(monster_glow_path(&hunter, &inventory).unwrap().is_none());
    }

    #[test]
    fn monster_glow_uses_selected_build_glow_from_imported_dds_pak() {
        // A normal extracted CDF is not necessarily inside a managed
        // archive/files directory. The caller's explicit same-build catalog
        // must still locate its variant glow without searching other builds.
        let temp = tempfile::tempdir().unwrap();
        let source_root = temp.path().join("extracted");
        let monster = source_root.join("characters/monsters/goliath");
        fs::create_dir_all(&monster).unwrap();
        let cdf = monster.join("goliath3.cdf");
        fs::write(&cdf, b"original selected CDF").unwrap();
        let library = temp.path().join("library");
        let stage = library.join("dds_low.pak__11111111111111111111");
        let legacy = library.join("legacy.pak__22222222222222222222");
        let name = "characters/monsters/goliath/goliath3_glow_diff.dds.0";
        let stage_dds = stage.join("files").join(name);
        let legacy_dds = legacy.join("files").join(name);
        for (dir, file, archive_name) in [
            (&stage, &stage_dds, "dds_low.pak"),
            (&legacy, &legacy_dds, "legacy.pak"),
        ] {
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, b"original").unwrap();
            fs::write(
                dir.join("asset-library.json"),
                serde_json::to_vec(&serde_json::json!({
                    "version": 1,
                    "name": archive_name,
                    "source": "C:/game/game.pak",
                    "entries": [{"path": name, "bytes": 8}]
                }))
                .unwrap(),
            )
            .unwrap();
        }
        fs::write(
            library.join("build-collections.json"),
            serde_json::to_vec(&serde_json::json!({
                "version":1,
                "builds":{
                    "stage-two":{"label":"Stage 2",
                        "members":{"11111111111111111111":{}}},
                    "legacy":{"label":"Legacy",
                        "members":{"22222222222222222222":{}}}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let inventory = crate::scan_directory(&source_root).unwrap();
        assert!(
            monster_glow_path(&cdf, &inventory).unwrap().is_none(),
            "No source glow exists in the local CDF directory"
        );
        let selected =
            monster_glow_path_with_library(&cdf, &inventory, Some(&library), Some("Stage 2"))
                .unwrap()
                .unwrap();
        assert_eq!(
            fs::canonicalize(selected).unwrap(),
            fs::canonicalize(stage_dds).unwrap()
        );
    }

    #[test]
    fn flat_monster_texture_paks_find_their_own_original_glow() {
        // The real Grounder/Kraken TS PAK stores grounder_glow_diff.dds.0
        // through .7 at its /files root, not in characters/monsters/grounder.
        // The Grounder3 CDF stem therefore cannot serve as the sole prefix.
        let temp = tempfile::tempdir().unwrap();
        let source_root = temp.path().join("extracted");
        let library = temp.path().join("library");
        let foreign = library.join("characters_monsters_rocky_ts.pak__22222222222222222222");
        let old = library.join("characters_monsters_grounder_ts.pak__33333333333333333333");
        let cases = [
            ("grounder", "grounder3", "grounder_glow_diff.dds.0"),
            ("teleporter", "teleporter3", "teleporter_glow_diff.dds.0"),
        ];
        let mut stage_ids = serde_json::Map::new();
        let mut legacy_ids = serde_json::Map::new();
        for (index, (owner, cdf_name, texture)) in cases.iter().enumerate() {
            let folder = source_root.join(format!("characters/monsters/{owner}"));
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join(format!("{cdf_name}.cdf")), b"source cdf").unwrap();

            let id = format!("{:020}", index + 1);
            let archive = library.join(format!("characters_monsters_{owner}_ts.pak__{id}"));
            let name = format!("characters_monsters_{owner}_ts.pak");
            let image = archive.join("files").join(texture);
            fs::create_dir_all(image.parent().unwrap()).unwrap();
            fs::write(&image, b"low fragment").unwrap();
            // High numbered fragments are part of the same source texture.
            for mip in 1..=7 {
                fs::write(
                    image.with_file_name(format!("{owner}_glow_diff.dds.{mip}")),
                    b"hi",
                )
                .unwrap();
            }
            let entries = (0..=7)
                .map(|n| {
                    serde_json::json!({
                        "path": format!("{owner}_glow_diff.dds.{n}"),
                        "bytes": if n == 0 { 12 } else { 2 }
                    })
                })
                .collect::<Vec<_>>();
            fs::write(
                archive.join("asset-library.json"),
                serde_json::to_vec(&serde_json::json!({
                    "version":1, "name":name, "source":"C:/test/game.pak", "entries":entries
                }))
                .unwrap(),
            )
            .unwrap();
            stage_ids.insert(id, serde_json::json!({}));
        }
        // Same name from another actor's TS archive and a different build
        // must NOT be silently used, even though the filename is identical.
        for (directory, archive_name, id) in [
            (
                &foreign,
                "characters_monsters_rocky_ts.pak",
                "22222222222222222222",
            ),
            (
                &old,
                "characters_monsters_grounder_ts.pak",
                "33333333333333333333",
            ),
        ] {
            fs::create_dir_all(directory.join("files")).unwrap();
            fs::write(directory.join("files/grounder_glow_diff.dds.0"), b"wrong").unwrap();
            fs::write(
                directory.join("asset-library.json"),
                serde_json::to_vec(&serde_json::json!({
                    "version":1,"name":archive_name,"source":"C:/test/other.pak",
                    "entries":[{"path":"grounder_glow_diff.dds.0","bytes":5}]
                }))
                .unwrap(),
            )
            .unwrap();
            if id == "33333333333333333333" {
                legacy_ids.insert(id.into(), serde_json::json!({}));
            } else {
                stage_ids.insert(id.into(), serde_json::json!({}));
            }
        }
        fs::write(
            library.join("build-collections.json"),
            serde_json::to_vec(&serde_json::json!({
                "version":1,
                "builds":{
                    "stage-two":{"label":"Stage 2","members":stage_ids},
                    "legacy":{"label":"Legacy","members":legacy_ids}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let inventory = crate::scan_directory(&source_root).unwrap();
        for (owner, stem, texture) in cases {
            let cdf = source_root.join(format!("characters/monsters/{owner}/{stem}.cdf"));
            assert!(monster_glow_path(&cdf, &inventory).unwrap().is_none());
            let result =
                monster_glow_path_with_library(&cdf, &inventory, Some(&library), Some("Stage 2"))
                    .unwrap()
                    .unwrap();
            assert_eq!(result.file_name().unwrap(), texture);
            assert!(
                result
                    .to_string_lossy()
                    .contains(&format!("characters_monsters_{owner}_ts.pak"))
            );
        }
        let grounder = source_root.join("characters/monsters/grounder/grounder3.cdf");
        // Legacy has an identically named path, but source-build selection
        // keeps it distinct from Stage 2.
        let legacy =
            monster_glow_path_with_library(&grounder, &inventory, Some(&library), Some("Legacy"))
                .unwrap()
                .unwrap();
        assert!(legacy.to_string_lossy().contains("33333333333333333333"));
        assert!(
            monster_glow_path_with_library(&grounder, &inventory, Some(&library), None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn cdf_resolves_scoped_high_dds_from_another_imported_pak() {
        let library = tempfile::tempdir().unwrap();
        let objects = library.path().join("Objects.pak__11111111111111111111");
        let high = library.path().join("dds_high.pak__22222222222222222222");
        let legacy = library.path().join("legacy_high.pak__33333333333333333333");
        let character = "Objects/characters/monsters/scorpid_2";
        for archive in [&objects, &high, &legacy] {
            fs::create_dir_all(archive.join("files").join(character)).unwrap();
        }
        let mesh = objects.join("files").join(character).join("scorpid3.chr");
        fs::write(&mesh, b"original model file").unwrap();
        let mtl = objects.join("files").join(character).join("scorpid3.mtl");
        fs::write(
            &mtl,
            br#"<Material Name="Scorpid"><Textures><Texture Map="Diffuse" File="body.tif"/></Textures></Material>"#,
        )
        .unwrap();
        let texture = format!("{character}/body.dds.0");
        let high_dds = high.join("files").join(&texture);
        fs::write(&high_dds, b"source high").unwrap();
        fs::write(legacy.join("files").join(&texture), b"legacy high").unwrap();
        for (archive, name, entries) in [
            (
                &objects,
                "Objects.pak",
                vec![
                    serde_json::json!({"path":format!("{character}/scorpid3.chr"),"bytes":19}),
                    serde_json::json!({"path":format!("{character}/scorpid3.mtl"),"bytes":fs::metadata(&mtl).unwrap().len()}),
                ],
            ),
            (
                &high,
                "dds_high.pak",
                vec![serde_json::json!({"path":texture,"bytes":11})],
            ),
            (
                &legacy,
                "legacy_high.pak",
                vec![serde_json::json!({"path":texture,"bytes":11})],
            ),
        ] {
            fs::write(
                archive.join("asset-library.json"),
                serde_json::to_vec(&serde_json::json!({
                    "version":1,"name":name,"source":"C:/game/assets.pak","entries":entries
                }))
                .unwrap(),
            )
            .unwrap();
        }
        fs::write(
            library.path().join("build-collections.json"),
            serde_json::to_vec(&serde_json::json!({
                "version":1,"builds":{
                    "stage-two":{"label":"Stage 2","members":{
                        "11111111111111111111":{},"22222222222222222222":{}
                    }},
                    "legacy":{"label":"Legacy","members":{"33333333333333333333":{}}}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let inventory = crate::scan_directory(&objects.join("files")).unwrap();
        let scope = texture_scope(&inventory).unwrap();
        assert_eq!(scope.build.as_deref(), Some("Stage 2"));
        let maps = exact_single_slot_maps_with_scope(
            Path::new(&inventory.root),
            &mesh,
            &inventory,
            None,
            Some(&scope),
        )
        .unwrap();
        assert_eq!(
            fs::canonicalize(maps.diffuse.unwrap()).unwrap(),
            fs::canonicalize(high_dds).unwrap()
        );
    }

    #[test]
    fn scorpid_low_tail_and_high_top_mip_use_same_build_manifest_only() {
        let lib = tempfile::tempdir().unwrap();
        let low = lib.path().join("dds_low.pak__11111111111111111111");
        let high = lib.path().join("dds_high.pak__22222222222222222222");
        let legacy = lib.path().join("legacy_high.pak__33333333333333333333");
        let relative = "objects/characters/monsters/scorpid_2/scorpid_body.dds";
        let low_file = low.join("files").join(format!("{relative}.0"));
        let high_file = high.join("files").join(format!("{relative}.2"));
        let legacy_file = legacy.join("files").join(format!("{relative}.2"));
        for file in [&low_file, &high_file, &legacy_file] {
            fs::create_dir_all(file.parent().unwrap()).unwrap();
        }
        let mut tail = vec![0u8; 128];
        tail[0..4].copy_from_slice(b"DDS ");
        tail[4..8].copy_from_slice(&124u32.to_le_bytes());
        tail[8..12].copy_from_slice(&0x21007u32.to_le_bytes());
        tail[12..16].copy_from_slice(&8u32.to_le_bytes());
        tail[16..20].copy_from_slice(&8u32.to_le_bytes());
        tail[28..32].copy_from_slice(&3u32.to_le_bytes());
        tail[76..80].copy_from_slice(&32u32.to_le_bytes());
        tail[80..84].copy_from_slice(&4u32.to_le_bytes());
        tail[84..88].copy_from_slice(b"DXT1");
        tail.extend_from_slice(&[224, 7, 0, 248, 0, 0, 0, 0]);
        fs::write(&low_file, &tail).unwrap();
        fs::write(
            low_file.with_file_name("scorpid_body.dds.1"),
            [224, 7, 0, 248, 0, 0, 0, 0],
        )
        .unwrap();
        fs::write(&high_file, [0, 248, 224, 7, 0, 0, 0, 0].repeat(4)).unwrap();
        fs::write(&legacy_file, [224, 7, 0, 248, 0, 0, 0, 0].repeat(4)).unwrap();
        for (archive, name, entries) in [
            (
                &low,
                "dds_low.pak",
                vec![
                    serde_json::json!({"path":format!("{relative}.0"),"bytes":136}),
                    serde_json::json!({"path":format!("{relative}.1"),"bytes":8}),
                ],
            ),
            (
                &high,
                "dds_high.pak",
                vec![serde_json::json!({"path":format!("{relative}.2"),"bytes":32})],
            ),
            (
                &legacy,
                "legacy_high.pak",
                vec![serde_json::json!({"path":format!("{relative}.2"),"bytes":32})],
            ),
        ] {
            fs::write(
                archive.join("asset-library.json"),
                serde_json::to_vec(&serde_json::json!({
                    "version":1,"name":name,"source":"C:/game/assets.pak","entries":entries
                }))
                .unwrap(),
            )
            .unwrap();
        }
        fs::write(
            lib.path().join("build-collections.json"),
            serde_json::to_vec(&serde_json::json!({
                "version":1,"builds":{
                    "stage-two":{"label":"Stage 2","members":{
                        "11111111111111111111":{},"22222222222222222222":{}
                    }},
                    "legacy":{"label":"Legacy","members":{"33333333333333333333":{}}}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let inventory = crate::scan_directory(&low.join("files")).unwrap();
        let scope = texture_scope(&inventory).unwrap();
        let fragments = matching_imported_high_mips(&low_file, Some(&scope)).unwrap();
        assert_eq!(fragments.len(), 1);
        assert_eq!(fragments[0], fs::canonicalize(&high_file).unwrap());
        assert!(crate::dds::decode_dds_rgba(&low_file).is_err());
        let (info, image) = decode_original_mip(&low_file, Some(&scope)).unwrap();
        assert_eq!(info.streaming_parts, 3);
        assert_eq!(image.get_pixel(7, 7).0, [255, 0, 0, 255]);
    }

    #[test]
    fn missing_original_2012_part_diffuse_must_not_be_called_textured() {
        let slots = vec![
            OriginalMaterialPng {
                name: "2012 skeleton".into(),
                no_draw: true,
                ..Default::default()
            },
            OriginalMaterialPng {
                name: "Scorpid_Body".into(),
                ..Default::default()
            },
        ];
        assert!(
            verify_embedded_source_diffuse("torso", &[(0, 3, 1)], &slots)
                .unwrap_err()
                .contains("Scorpid_Body")
        );
        // An unused skeleton leaf must not replace the actual body map.
        assert!(verify_embedded_source_diffuse("torso", &[(0, 3, 0), (3, 3, 1)], &slots).is_err());
        let mut textured = slots;
        textured[1].diffuse = Some(b"original diffuse PNG".to_vec());
        assert!(
            verify_embedded_source_diffuse("torso", &[(0, 3, 0), (3, 3, 1)], &textured).is_ok()
        );
        assert!(verify_embedded_source_diffuse("torso", &[(0, 3, 2)], &textured).is_err());
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
