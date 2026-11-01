//! Exact, read-only local and imported-library dependency discovery for the
//! model viewer. Never infer an animation from a matching filename alone.
use evolve_core::{
    AnimationEventDatabase, AssetResolution, Inventory, LibraryReferenceStatus,
    animation_references, animation_wildcard_matches, imported_library, read_animation_events,
    resolve_imported_reference, resolve_virtual,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct AutoClip {
    pub label: String,
    pub kind: String,
    pub virtual_path: String,
    pub source: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct AutoDiscovery {
    pub material: Option<PathBuf>,
    pub skeleton: Option<PathBuf>,
    /// Unique authored CDF that owns this standalone skin/equipment mesh.
    /// Used to preview complete skeleton-bound character assemblies rather
    /// than leaving an isolated equipment arm in raw mesh coordinates.
    pub owning_cdf: Option<String>,
    pub material_note: String,
    pub clips: Vec<AutoClip>,
    pub idle: Option<PathBuf>,
    pub config: Option<PathBuf>,
    /// Exact `$AnimEventDatabase` declaration and bounded read-only event index.
    pub event_database: Option<(String, AnimationEventDatabase)>,
    pub warnings: Vec<String>,
}

/// Local entries have precedence only when the exact virtual path is unique.
/// A saved library hit must still be manifest-verified by the core resolver.
pub(crate) fn exact(
    inv: &Inventory,
    virtual_name: &str,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    match resolve_virtual(inv, virtual_name) {
        AssetResolution::Found(relative) => {
            let root = std::fs::canonicalize(&inv.root).map_err(|e| e.to_string())?;
            let file = PathBuf::from(&inv.root).join(&relative);
            let meta = std::fs::symlink_metadata(&file).map_err(|e| e.to_string())?;
            let canonical = std::fs::canonicalize(&file).map_err(|e| e.to_string())?;
            // Read-only preview inventories may defer checks on unrelated
            // members. Verify the actual requested member against its manifest
            // byte length as well as canonical path before opening it.
            let expected = inv
                .entries
                .iter()
                .find(|entry| entry.relative_path == relative)
                .map(|entry| entry.byte_size);
            if !meta.file_type().is_file()
                || !canonical.starts_with(root)
                || expected != Some(meta.len())
            {
                return Err(format!("Unsafe or modified indexed asset: {virtual_name}"));
            }
            Ok(Some(canonical))
        }
        AssetResolution::Ambiguous(_) => Err(format!("Ambiguous indexed asset: {virtual_name}")),
        AssetResolution::Missing => {
            let Some(root) = imported else {
                return Ok(None);
            };
            match resolve_imported_reference(root, virtual_name, build)
                .map_err(|e| e.to_string())?
            {
                LibraryReferenceStatus::Missing => Ok(None),
                LibraryReferenceStatus::Ambiguous(_) => {
                    Err(format!("Ambiguous imported asset: {virtual_name}"))
                }
                LibraryReferenceStatus::Found(hit) => {
                    let root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
                    let path = PathBuf::from(hit.absolute_path);
                    let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                    let canonical = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
                    if !meta.file_type().is_file() || !canonical.starts_with(root) {
                        return Err(format!("Unsafe imported asset: {virtual_name}"));
                    }
                    Ok(Some(canonical))
                }
            }
        }
    }
}

/// CryEngine .skin/.chr are authored skeleton bindings. Their render
/// companions may be in another imported shared PAK; resolve only the exact
/// virtual companion path, never a similarly named mesh from another actor.
pub fn exact_render_companion(
    relative: &str,
    inventory: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let lower = relative.to_ascii_lowercase();
    let target = [".skin", ".chr", ".cgf", ".cga"]
        .iter()
        .any(|extension| lower.ends_with(extension))
        .then(|| format!("{relative}m"));
    match target {
        Some(name) => exact(inventory, &name, imported, build),
        None => Ok(None),
    }
}

/// Resolve an arbitrary compiled skin to its CDF by actual authored Binding,
/// rather than assuming the CDF has the same filename as a torso or weapon.
/// Only safely indexed local definitions are searched; imported archives
/// require separately verified provenance and are never scanned by basename.
fn declaring_local_cdf(
    relative: &str,
    inv: &Inventory,
) -> Result<Option<(String, evolve_core::CdfDefinition)>, String> {
    let lower = relative.replace('\\', "/").to_ascii_lowercase();
    let binding = [
        (".skinm", ".skin"),
        (".chrm", ".chr"),
        (".cgfm", ".cgf"),
        (".cgam", ".cga"),
    ]
    .iter()
    .find_map(|(compiled, authored)| {
        lower
            .strip_suffix(compiled)
            .map(|base| format!("{base}{authored}"))
    })
    .unwrap_or(lower);
    let cdfs = inv
        .entries
        .iter()
        .filter(|entry| entry.relative_path.to_ascii_lowercase().ends_with(".cdf"))
        .collect::<Vec<_>>();
    if cdfs.len() > 2048 {
        return Err("Too many indexed CDFs for bounded ownership discovery".into());
    }
    let root = std::fs::canonicalize(&inv.root).map_err(|e| e.to_string())?;
    let mut inspected_bytes = 0u64;
    let mut owner = None;
    for entry in cdfs {
        let path = root.join(&entry.relative_path);
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !meta.file_type().is_file() || meta.len() > 2 * 1024 * 1024 {
            continue;
        }
        inspected_bytes = inspected_bytes
            .checked_add(meta.len())
            .ok_or("CDF ownership discovery byte budget overflow")?;
        if inspected_bytes > 128 * 1024 * 1024 {
            return Err("CDF ownership discovery exceeds the 128 MiB read budget".into());
        }
        let verified = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
        if !verified.starts_with(&root) {
            return Err(format!("Unsafe indexed CDF: {}", entry.relative_path));
        }
        let Ok(cdf) = evolve_core::read_cdf(&verified) else {
            continue;
        };
        let belongs = cdf.skeleton.eq_ignore_ascii_case(&binding)
            || cdf
                .attachments
                .iter()
                .any(|attachment| attachment.binding.eq_ignore_ascii_case(&binding));
        if !belongs {
            continue;
        }
        if owner.is_some() {
            return Err(format!(
                "Multiple authored CDFs reference {binding}; select the intended CDF assembly"
            ));
        }
        owner = Some((entry.relative_path.clone(), cdf));
    }
    Ok(owner)
}

fn stems(name: &str) -> Vec<String> {
    let name = name.rsplit('/').next().unwrap_or(name);
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let mut variants = vec![stem.to_string()];
    let lower = stem.to_ascii_lowercase();
    let stripped_lod = lower
        .rfind("_lod")
        .and_then(|at| {
            lower[at + 4..]
                .chars()
                .all(|c| c.is_ascii_digit())
                .then_some(&stem[..at])
        })
        .unwrap_or(stem);
    if stripped_lod != stem {
        variants.push(stripped_lod.to_owned());
    }
    for ending in ["_3p", "_fp", "_skeleton"] {
        if stripped_lod.to_ascii_lowercase().ends_with(ending) {
            variants.push(stripped_lod[..stripped_lod.len() - ending.len()].to_owned());
        }
    }
    variants.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    variants
}

fn join(parent: &str, name: &str, ext: &str) -> String {
    if parent.is_empty() {
        format!("{name}.{ext}")
    } else {
        format!("{parent}/{name}.{ext}")
    }
}

fn idle_priority(name: &str) -> Option<(bool, bool, bool, String)> {
    let n = name.to_ascii_lowercase();
    if !n
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| part == "idle")
    {
        return None;
    }
    Some((
        [
            "additive",
            "underwater",
            "swim",
            "incap",
            "jetpack",
            "aim",
            "fp_",
        ]
        .iter()
        .any(|p| n.contains(p)),
        !["stand", "neutral", "relaxed"]
            .iter()
            .any(|p| n.contains(p)),
        !n.contains("loop"),
        n,
    ))
}

/// Expand only source-declared wildcard CAFs. Local entries and matching-build
/// AnimationsCAF manifests are metadata; each clip is independently verified
/// by exact() when selected. Never walk arbitrary game directories or borrow
/// a different build's animation by basename.
/// Collect source-declared CAF candidates only once per model-resource discovery.
/// A character can declare many wildcards; rereading and parsing the same large
/// AnimationsCAF.pak manifest for each wildcard makes every model open expensive.
fn archive_animation_catalog_paths(
    inv: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
    archive_name: &str,
    extension: &str,
) -> Result<Vec<String>, String> {
    const MAX_MANIFEST: u64 = 32 * 1024 * 1024;
    let mut paths = std::collections::BTreeMap::<String, String>::new();
    let mut consider = |path: &str| -> Result<(), String> {
        let normalized = path.replace('\\', "/");
        if normalized.to_ascii_lowercase().ends_with(extension) {
            paths
                .entry(normalized.to_ascii_lowercase())
                .or_insert(normalized);
            if paths.len() > 250_000 {
                return Err("Animation catalog exceeds 250000 distinct paths".into());
            }
        }
        Ok(())
    };
    for entry in &inv.entries {
        consider(&entry.relative_path)?;
    }
    if let Some(root) = imported {
        let catalog = imported_library(root).map_err(|e| e.to_string())?;
        for archive in &catalog.archives {
            if !archive.name.eq_ignore_ascii_case(archive_name)
                || build.is_some_and(|label| !archive.builds.iter().any(|b| b == label))
            {
                continue;
            }
            let manifest = Path::new(&archive.directory).join("asset-library.json");
            let meta = std::fs::symlink_metadata(&manifest).map_err(|e| e.to_string())?;
            if !meta.file_type().is_file() || meta.len() > MAX_MANIFEST {
                return Err("Unsafe animation archive manifest".into());
            }
            let raw = std::fs::read(&manifest).map_err(|e| e.to_string())?;
            let document: serde_json::Value =
                serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
            if document["version"].as_u64() != Some(1)
                || document["name"]
                    .as_str()
                    .is_none_or(|s| !s.eq_ignore_ascii_case(&archive.name))
            {
                return Err("Animation archive manifest identity mismatch".into());
            }
            let entries = document["entries"]
                .as_array()
                .ok_or("Missing animation archive entries")?;
            if entries.len() > 250_000 {
                return Err("Animation archive manifest member limit".into());
            }
            for entry in entries {
                if let Some(name) = entry["path"].as_str() {
                    consider(name)?;
                }
            }
        }
    }
    Ok(paths.into_values().collect())
}

fn caf_catalog_paths(
    inv: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<Vec<String>, String> {
    archive_animation_catalog_paths(inv, imported, build, "AnimationsCAF.pak", ".caf")
}

fn dba_catalog_paths(
    inv: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<Vec<String>, String> {
    archive_animation_catalog_paths(inv, imported, build, "AnimationsDBA.pak", ".dba")
}

/// Expand an authored wildcard against the already verified exact-build
/// candidates, not by rereading the source archive for each pattern.
fn matching_caf_paths(pattern: &str, candidates: &[String]) -> Result<Vec<String>, String> {
    const MAX_MATCHES: usize = 12_000;
    let mut matches = Vec::new();
    for path in candidates {
        if animation_wildcard_matches(pattern, path) {
            matches.push(path.clone());
            if matches.len() > MAX_MATCHES {
                return Err("Animation wildcard exceeds 12000 matching paths".into());
            }
        }
    }
    Ok(matches)
}

pub fn discover(
    relative: &str,
    inv: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
) -> AutoDiscovery {
    let mut found = AutoDiscovery::default();
    let canonical = relative.replace('\\', "/");
    // A CDF is a character, not a renderable mesh. Follow its explicit Model
    // File skeleton to discover the correct chrparams/idle instead of trying
    // to find animation files named after the CDF itself.
    let mut source = canonical.clone();
    if canonical.to_ascii_lowercase().ends_with(".cdf") {
        match exact(inv, &canonical, imported, build) {
            Ok(Some(cdf_path)) => match evolve_core::read_cdf(&cdf_path) {
                Ok(cdf) => {
                    source = cdf.skeleton.clone();
                    match exact(inv, &cdf.skeleton, imported, build) {
                        Ok(Some(skeleton)) => found.skeleton = Some(skeleton),
                        Ok(None) => found
                            .warnings
                            .push(format!("CDF compiled skeleton not found: {}", cdf.skeleton)),
                        Err(error) => found.warnings.push(error),
                    }
                }
                Err(error) => found
                    .warnings
                    .push(format!("Cannot read selected CDF: {error}")),
            },
            Ok(None) => found
                .warnings
                .push("CDF missing from indexed assets".into()),
            Err(error) => found.warnings.push(error),
        }
    }
    let mut animation_source = source.clone();
    let mut animation_binding_ambiguous = false;
    // Body, equipment, and compiled skin filenames often differ from the
    // owning CDF. Use its exact Binding references for skeleton/CAF lookup.
    // The selected mesh retains its own material discovery path.
    if !canonical.to_ascii_lowercase().ends_with(".cdf")
        && [
            ".skin", ".skinm", ".chr", ".chrm", ".cgf", ".cgfm", ".cga", ".cgam",
        ]
        .iter()
        .any(|ext| canonical.to_ascii_lowercase().ends_with(ext))
    {
        match declaring_local_cdf(&canonical, inv) {
            Ok(Some((owner, cdf))) => {
                found.owning_cdf = Some(owner);
                animation_source = cdf.skeleton.clone();
                match exact(inv, &cdf.skeleton, imported, build) {
                    Ok(Some(skeleton)) => found.skeleton = Some(skeleton),
                    Ok(None) => found.warnings.push(format!(
                        "Declaring CDF compiled skeleton not found: {}",
                        cdf.skeleton
                    )),
                    Err(error) => found.warnings.push(error),
                }
            }
            Ok(None) => {}
            Err(error) => {
                found.warnings.push(error);
                animation_binding_ambiguous = true;
            }
        }
    }
    let parent = source.rsplit_once('/').map_or("", |(p, _)| p);
    let bases = stems(&source);
    // Original CrChF MtlName references take precedence over guessed same-stem
    // MTLs. Reuse the CDF loader's exact source/build-checked resolver so a
    // separately named material never silently becomes the wrong character.
    let mut fallback_material = true;
    if [".skin", ".skinm", ".chr", ".chrm", ".cgf", ".cga"]
        .iter()
        .any(|ext| canonical.to_ascii_lowercase().ends_with(ext))
    {
        match crate::assembly_materials::material_for_part(&canonical, None, inv, imported, build) {
            Ok(Some(path)) => {
                found.material_note = "Verified original compiled MtlName or exact material".into();
                found.material = Some(path);
            }
            Ok(None) => {}
            Err(error) => {
                found.warnings.push(error);
                fallback_material = false;
            }
        }
    }
    // Same-folder material names are an explicit fallback, not a general
    // basename search across characters or a random library texture.
    if found.material.is_none() && fallback_material {
        for base in &bases {
            let candidate = join(parent, base, "mtl");
            match exact(inv, &candidate, imported, build) {
                Ok(Some(path)) => {
                    found.material = Some(path);
                    found.material_note = format!("Exact material: {candidate}");
                    break;
                }
                Err(e) => {
                    found.warnings.push(e);
                    // A conflicted higher-priority reference cannot be bypassed.
                    break;
                }
                Ok(None) => {}
            }
        }
    }
    if found.material.is_none() && found.warnings.is_empty() {
        found.material_note = "No verified material for this mesh".into();
    }
    // A shared skin claimed by multiple CDFs has no unique verified rig.
    // Preserve independent mesh/MTL preview but never attach an arbitrary
    // similarly named animation catalog to that ambiguous character.
    if animation_binding_ambiguous {
        return found;
    }
    // Only this character's matching skeleton animation configuration.
    let animation_parent = animation_source
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent);
    let animation_bases = stems(&animation_source);
    let mut configs = animation_bases.clone();
    for base in &animation_bases {
        if !base.to_ascii_lowercase().ends_with("_skeleton") {
            configs.push(format!("{base}_skeleton"));
        }
    }
    configs.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    for stem in configs {
        let name = join(animation_parent, &stem, "chrparams");
        match exact(inv, &name, imported, build) {
            Ok(Some(path)) => {
                found.config = Some(path);
                break;
            }
            Err(e) => {
                found.warnings.push(e);
                break;
            }
            Ok(None) => {}
        }
    }
    let Some(config) = &found.config else {
        return found;
    };
    let data = match std::fs::metadata(config).and_then(|m| {
        if m.len() > 5 * 1024 * 1024 {
            Err(std::io::Error::other("chrparams exceeds safe size"))
        } else {
            std::fs::read(config)
        }
    }) {
        Ok(raw) => raw,
        Err(e) => {
            found
                .warnings
                .push(format!("Could not read character animation catalog: {e}"));
            return found;
        }
    };
    let references = match animation_references(&data) {
        Ok(refs) => refs,
        Err(e) => {
            found
                .warnings
                .push(format!("Invalid character animation catalog: {e}"));
            return found;
        }
    };
    // Keep the complete declared catalog in the browser (bounded by parser).
    // Only the best neutral idle is materialized from the imported library by
    // default; clicking a different clip resolves it on demand.
    // Lazily decode the large animation archive manifest once for all CAF wildcards.
    let mut caf_candidates: Option<Result<Vec<String>, String>> = None;
    let mut dba_candidates: Option<Result<Vec<String>, String>> = None;
    let mut preferred: Vec<(usize, (bool, bool, bool, String))> = Vec::new();
    let mut declared_event_databases = 0;
    for r in &references {
        if r.kind == "animevents" {
            declared_event_databases += 1;
            if declared_event_databases > 1 {
                if declared_event_databases == 2 {
                    found.warnings.push(
                        "Multiple declared animation event databases; events disabled".into(),
                    );
                }
                found.event_database = None;
                continue;
            }
            match exact(inv, &r.path, imported, build) {
                Ok(Some(path)) => match read_animation_events(path) {
                    Ok(events) => {
                        if let Some(diagnostic) = events.diagnostics().first() {
                            found.warnings.push(format!(
                                "Authored event warnings ({} total): {diagnostic}",
                                events.diagnostics().len()
                            ));
                        }
                        found.event_database = Some((r.path.clone(), events));
                    }
                    Err(error) => found
                        .warnings
                        .push(format!("Declared event database invalid: {error}")),
                },
                Ok(None) => found
                    .warnings
                    .push(format!("Declared event database unavailable: {}", r.path)),
                Err(error) => found.warnings.push(error),
            }
            continue;
        }
        if r.kind == "caf_glob" {
            let candidates =
                caf_candidates.get_or_insert_with(|| caf_catalog_paths(inv, imported, build));
            match candidates
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|paths| matching_caf_paths(&r.path, paths))
            {
                Ok(paths) => {
                    for path in paths {
                        let label = path
                            .rsplit('/')
                            .next()
                            .unwrap_or(&path)
                            .trim_end_matches(".caf")
                            .to_owned();
                        found.clips.push(AutoClip {
                            label,
                            kind: "caf".into(),
                            virtual_path: path,
                            source: None,
                        });
                    }
                }
                Err(error) => found.warnings.push(format!("CAF wildcard: {error}")),
            }
            continue;
        }
        if r.kind == "dba_glob" && r.database {
            let candidates =
                dba_candidates.get_or_insert_with(|| dba_catalog_paths(inv, imported, build));
            match candidates
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|paths| matching_caf_paths(&r.path, paths))
            {
                Ok(paths) => {
                    for path in paths {
                        found.clips.push(AutoClip {
                            label: r.name.clone(),
                            kind: "dba".into(),
                            virtual_path: path,
                            source: None,
                        });
                    }
                }
                Err(error) => found.warnings.push(format!("DBA wildcard: {error}")),
            }
            continue;
        }
        let path = if r.kind == "caf" {
            match exact(inv, &r.path, None, None) {
                Ok(path) => path,
                Err(e) => {
                    found.warnings.push(e);
                    None
                }
            }
        } else {
            None
        };
        if r.kind == "caf"
            && let Some(rank) = idle_priority(&r.name)
        {
            preferred.push((found.clips.len(), rank));
        }
        found.clips.push(AutoClip {
            label: r.name.clone(),
            kind: r.kind.clone(),
            virtual_path: r.path.clone(),
            source: path,
        });
    }
    preferred.sort_by(|a, b| a.1.cmp(&b.1));
    for (index, rank) in preferred.into_iter().take(4) {
        if rank.0 {
            continue;
        } // additive/contextual idles are never auto-picked
        if found.clips[index].source.is_none() {
            match exact(inv, &found.clips[index].virtual_path, imported, build) {
                Ok(path) => found.clips[index].source = path,
                Err(e) => found.warnings.push(e),
            }
        }
        if let Some(path) = &found.clips[index].source {
            found.idle = Some(path.clone());
            break;
        }
    }
    found
}

/// Resolve one explicitly declared CAF on user request. Imported-library
/// manifests are rechecked here instead of reusing a possibly stale path.
pub fn resolve_declared(
    inventory: &Inventory,
    declared_path: &str,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    if !declared_path.to_ascii_lowercase().ends_with(".caf") {
        return Err("Only declared CAF paths can be loaded this way".into());
    }
    exact(inventory, declared_path, imported, build)
}

/// Packed DBA directories identify the original virtual CAF *path*, not
/// just a similarly named animation in another hunter or monster. Some
/// Controller_905 records omit the .caf suffix; that is the only normalization
/// permitted here. Never select from the basename alone.
fn packed_animation_key(value: &str) -> String {
    let normalized = value.replace('\\', "/").to_ascii_lowercase();
    normalized
        .trim_start_matches('/')
        .trim_end_matches(".caf")
        .to_owned()
}

pub(crate) fn packed_record_matches(declared_caf: &str, record: &str) -> bool {
    let wanted = packed_animation_key(declared_caf);
    let actual = packed_animation_key(record);
    !wanted.is_empty() && wanted.contains('/') && actual == wanted
}

/// Look only in exact DBAs declared by the selected character's CHRPARAMS.
/// The current Controller_905 parser validates the database's animation-name
/// directory but does NOT decode compressed keyframes. A positive result is a
/// converter candidate, never proof of native CAF or skeleton playback.
pub fn find_declared_packed_caf(
    inventory: &Inventory,
    requested_caf: &str,
    declared_databases: &[String],
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<Option<(PathBuf, String)>, String> {
    use evolve_core::index_native_dba_bytes;
    const MAX_DATABASES: usize = 64;
    const MAX_DBA: u64 = 512 * 1024 * 1024;
    if declared_databases.len() > MAX_DATABASES {
        return Err("Too many declared DBA databases to check safely".into());
    }
    let mut matched: Option<(PathBuf, String)> = None;
    let mut seen = std::collections::HashSet::new();
    for virtual_path in declared_databases {
        if !seen.insert(virtual_path.to_ascii_lowercase()) {
            continue;
        }
        let Some(path) = resolve_declared_dba(inventory, virtual_path, imported, build)? else {
            continue;
        };
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !meta.file_type().is_file() || meta.len() > MAX_DBA {
            return Err(format!("Unsafe or oversized declared DBA: {virtual_path}"));
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let records = index_native_dba_bytes(&bytes)
            .map_err(|e| format!("Cannot index declared DBA {virtual_path}: {e}"))?;
        for record in records {
            if !packed_record_matches(requested_caf, &record.name) {
                continue;
            }
            if matched.is_some() {
                return Err(format!(
                    "Animation {requested_caf} appears in multiple declared DBA records; refusing an ambiguous packed clip"
                ));
            }
            matched = Some((path.clone(), record.name));
        }
    }
    Ok(matched)
}

/// Resolve a DBA named in the selected character's own chrparams catalog.
/// Refuse arbitrary filenames and let the converter independently validate
/// its decoded GLB tracks before anything becomes playable.
pub fn resolve_declared_dba(
    inventory: &Inventory,
    declared_path: &str,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    if !declared_path.to_ascii_lowercase().ends_with(".dba") {
        return Err("Only explicitly declared DBA paths may be opened".into());
    }
    exact(inventory, declared_path, imported, build)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_core::scan_directory;

    #[test]
    fn packed_dba_matches_only_exact_character_animation_paths() {
        let path = "characters/hunters/merc_aberenegade/animations/3p/idle.caf";
        assert!(packed_record_matches(path, path));
        assert!(packed_record_matches(
            path,
            "Characters\\Hunters\\merc_aberenegade\\animations\\3p\\idle"
        ));
        assert!(!packed_record_matches(path, "idle.caf"));
        assert!(!packed_record_matches(
            path,
            "characters/hunters/merc_abe/animations/3p/idle.caf"
        ));
        assert!(!packed_record_matches(path, "animations/3p/idle.caf"));
    }

    #[test]
    fn packed_lookup_requires_declared_database_and_never_guesses_a_pak() {
        let dir = tempfile::tempdir().unwrap();
        let inv = scan_directory(dir.path()).unwrap();
        assert!(
            find_declared_packed_caf(
                &inv,
                "characters/hunters/merc_aberenegade/animations/3p/idle.caf",
                &[],
                None,
                None,
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn dba_catalog_paths_never_borrow_other_character_or_filename() {
        let dir = tempfile::tempdir().unwrap();
        for relative in [
            "characters/hunters/merc_aberenegade/animations/3p/abe.dba",
            "characters/hunters/merc_abe/animations/3p/abe.dba",
        ] {
            let path = dir.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"test").unwrap();
        }
        let inv = scan_directory(dir.path()).unwrap();
        let names = dba_catalog_paths(&inv, None, None).unwrap();
        assert_eq!(
            matching_caf_paths(
                "characters/hunters/merc_aberenegade/animations/3p/*.dba",
                &names,
            ).unwrap(),
            vec!["characters/hunters/merc_aberenegade/animations/3p/abe.dba"],
        );
    }

    #[test]
    fn wildcard_expands_only_matching_character_cafs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("characters/hunters/merc_bucket/animations"))
            .unwrap();
        std::fs::create_dir_all(dir.path().join("characters/hunters/merc_emet/animations"))
            .unwrap();
        std::fs::write(
            dir.path()
                .join("characters/hunters/merc_bucket/animations/bucket_idle.caf"),
            b"caf",
        )
        .unwrap();
        std::fs::write(
            dir.path()
                .join("characters/hunters/merc_emet/animations/emet_idle.caf"),
            b"caf",
        )
        .unwrap();
        let inv = scan_directory(dir.path()).unwrap();
        let candidates = caf_catalog_paths(&inv, None, None).unwrap();
        let clips = matching_caf_paths(
            "characters/hunters/merc_bucket/animations/*.caf",
            &candidates,
        )
        .unwrap();
        assert_eq!(
            matching_caf_paths("characters/hunters/merc_emet/animations/*.caf", &candidates)
                .unwrap(),
            vec!["characters/hunters/merc_emet/animations/emet_idle.caf"]
        );
        assert_eq!(
            clips,
            vec!["characters/hunters/merc_bucket/animations/bucket_idle.caf"]
        );
    }

    #[test]
    fn render_companion_never_falls_back_to_neighboring_character() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("characters")).unwrap();
        std::fs::write(
            dir.path().join("characters/hunter.skin"),
            b"source skeleton",
        )
        .unwrap();
        std::fs::write(dir.path().join("characters/monster.skinm"), b"wrong mesh").unwrap();
        let inventory = scan_directory(dir.path()).unwrap();
        assert!(
            exact_render_companion("characters/hunter.skin", &inventory, None, None)
                .unwrap()
                .is_none()
        );
        std::fs::write(
            dir.path().join("characters/hunter.skinm"),
            b"same virtual source",
        )
        .unwrap();
        let inventory = scan_directory(dir.path()).unwrap();
        let chosen = exact_render_companion("characters/hunter.skin", &inventory, None, None)
            .unwrap()
            .unwrap();
        assert_eq!(chosen.file_name().unwrap(), "hunter.skinm");
    }

    #[test]
    fn dba_resolution_never_opens_a_non_database_or_neighboring_file() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("animations/kala")).unwrap();
        std::fs::write(
            root.path().join("animations/kala/kala_3p.dba"),
            b"not validated by resolver",
        )
        .unwrap();
        std::fs::write(root.path().join("animations/kala/kala.caf"), b"other").unwrap();
        let inv = scan_directory(root.path()).unwrap();
        assert!(resolve_declared_dba(&inv, "animations/kala/kala.caf", None, None).is_err());
        let exact = resolve_declared_dba(&inv, "animations/kala/kala_3p.dba", None, None).unwrap();
        assert!(exact.unwrap().ends_with("kala_3p.dba"));
        assert!(
            resolve_declared_dba(&inv, "animations/other/other_3p.dba", None, None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn chooses_matching_character_not_neighboring_model() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("characters/kala")).unwrap();
        std::fs::write(root.path().join("characters/kala/kala.mtl"), b"<Material/>").unwrap();
        std::fs::write(
            root.path().join("characters/kala/other.mtl"),
            b"<Material/>",
        )
        .unwrap();
        let inv = scan_directory(root.path()).unwrap();
        let found = discover("characters/kala/kala_lod0.skin", &inv, None, None);
        assert!(found.material.unwrap().ends_with("kala.mtl"));
    }

    #[test]
    fn compiled_mtl_name_outweighs_neighboring_same_stem_material() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("characters/hunter");
        std::fs::create_dir_all(&folder).unwrap();
        let mut compiled = vec![0u8; 16 + 16 + 132];
        compiled[..8].copy_from_slice(b"CrChF\x07\0\0");
        compiled[8..12].copy_from_slice(&1u32.to_le_bytes());
        compiled[12..16].copy_from_slice(&16u32.to_le_bytes());
        compiled[16..20].copy_from_slice(&0x08021014u32.to_le_bytes());
        compiled[24..28].copy_from_slice(&132u32.to_le_bytes());
        compiled[28..32].copy_from_slice(&32u32.to_le_bytes());
        let authored = b"characters/hunter/true_material";
        compiled[32..32 + authored.len()].copy_from_slice(authored);
        std::fs::write(folder.join("torso.skinm"), compiled).unwrap();
        std::fs::write(folder.join("torso.mtl"), b"<Material/>").unwrap();
        std::fs::write(folder.join("true_material.mtl"), b"<Material/>").unwrap();
        let inventory = scan_directory(dir.path()).unwrap();
        let found = discover("characters/hunter/torso.skinm", &inventory, None, None);
        assert!(found.material.unwrap().ends_with("true_material.mtl"));
        assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    }

    #[test]
    fn missing_model_material_never_uses_unrelated_mtl() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("other.mtl"), b"<Material/>").unwrap();
        let inv = scan_directory(root.path()).unwrap();
        let found = discover("characters/kala/kala.skin", &inv, None, None);
        assert!(found.material.is_none());
        assert!(found.idle.is_none());
    }

    #[test]
    fn character_catalog_picks_exact_neutral_idle_and_keeps_other_declared_clips() {
        let temp = tempfile::tempdir().unwrap();
        let character = temp.path().join("characters/kala");
        let animations = character.join("animations/3p");
        std::fs::create_dir_all(&animations).unwrap();
        let catalog = br##"<Params><AnimationList>
            <Animation name="#filepath" path="characters/kala/animations/3p"/>
            <Animation name="$AnimEventDatabase" path="characters/kala/kala.animevents"/>
            <Animation name="swim_idle" path="swim_idle.caf"/>
            <Animation name="stand_idle_loop" path="stand_idle_loop.caf"/>
            <Animation name="walk_forward" path="walk_forward.caf"/>
        </AnimationList></Params>"##;
        std::fs::write(character.join("kala.chrparams"), catalog).unwrap();
        std::fs::write(character.join("kala.animevents"), br#"<anim_event_list><animation name="characters/kala/animations/3p/stand_idle_loop.caf"><event name="footstep" time="0.5"/></animation></anim_event_list>"#).unwrap();
        for clip in ["swim_idle.caf", "stand_idle_loop.caf", "walk_forward.caf"] {
            std::fs::write(animations.join(clip), b"fixture").unwrap();
        }
        let inventory = scan_directory(temp.path()).unwrap();
        let found = discover("characters/kala/kala.skin", &inventory, None, None);
        assert!(found.config.is_some());
        assert_eq!(found.clips.len(), 3);
        assert_eq!(found.clips[1].label, "stand_idle_loop");
        let (declared, events) = found.event_database.as_ref().unwrap();
        assert_eq!(declared, "characters/kala/kala.animevents");
        assert_eq!(
            events
                .timeline("characters/kala/animations/3p/stand_idle_loop.caf")
                .unwrap()
                .events
                .len(),
            1
        );
        assert!(found.idle.unwrap().ends_with("stand_idle_loop.caf"));
        assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    }

    #[test]
    fn standalone_skin_finds_original_differently_named_cdf_skeleton_and_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("characters/hunter");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("torso.skinm"), b"compiled geometry").unwrap();
        std::fs::write(folder.join("rig.chr"), b"compiled skeleton").unwrap();
        std::fs::write(
            folder.join("whole_hunter.cdf"),
            br#"<CharacterDefinition><Model File="characters/hunter/rig.chr"/>
                <AttachmentList><Attachment Type="CA_SKIN" AName="torso"
                    Binding="characters/hunter/torso.skin"/></AttachmentList>
                </CharacterDefinition>"#,
        )
        .unwrap();
        std::fs::write(
            folder.join("rig.chrparams"),
            br#"<Params><AnimationList></AnimationList></Params>"#,
        )
        .unwrap();
        let inventory = scan_directory(dir.path()).unwrap();
        let found = discover("characters/hunter/torso.skinm", &inventory, None, None);
        assert_eq!(
            found.owning_cdf.as_deref(),
            Some("characters/hunter/whole_hunter.cdf")
        );
        assert!(found.skeleton.unwrap().ends_with("rig.chr"));
        assert!(found.config.unwrap().ends_with("rig.chrparams"));
        assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    }

    #[test]
    fn overlapping_cdfs_do_not_guess_a_character_skeleton() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("characters/hunter");
        std::fs::create_dir_all(&folder).unwrap();
        for name in ["alternate.cdf", "default.cdf"] {
            std::fs::write(
                folder.join(name),
                br#"<CharacterDefinition><Model File="characters/hunter/rig.chr"/>
                    <AttachmentList><Attachment Type="CA_SKIN" AName="torso"
                        Binding="characters/hunter/torso.skin"/></AttachmentList>
                    </CharacterDefinition>"#,
            )
            .unwrap();
        }
        std::fs::write(
            folder.join("torso.chrparams"),
            br#"<Params><AnimationList></AnimationList></Params>"#,
        )
        .unwrap();
        let inventory = scan_directory(dir.path()).unwrap();
        let found = discover("characters/hunter/torso.skinm", &inventory, None, None);
        assert!(found.skeleton.is_none());
        assert!(found.config.is_none());
        assert!(
            found
                .warnings
                .iter()
                .any(|w| w.contains("Multiple authored CDFs"))
        );
    }

    #[test]
    fn declared_clip_requires_exact_safe_name() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("selected.caf"), b"fixture").unwrap();
        let inventory = scan_directory(temp.path()).unwrap();
        assert!(
            resolve_declared(&inventory, "selected.caf", None, None)
                .unwrap()
                .is_some()
        );
        assert!(
            resolve_declared(&inventory, "other.caf", None, None)
                .unwrap()
                .is_none()
        );
        assert!(
            resolve_declared(&inventory, "../selected.caf", None, None)
                .unwrap()
                .is_none()
        );
        assert!(resolve_declared(&inventory, "selected.dba", None, None).is_err());
    }

    #[test]
    fn idle_ranking_skips_contextual_motions() {
        assert!(idle_priority("stand_idle_loop").unwrap() < idle_priority("swim_idle").unwrap());
        assert_eq!(idle_priority("walk_forward"), None);
    }
}
