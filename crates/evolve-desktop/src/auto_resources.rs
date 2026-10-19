//! Exact, read-only local and imported-library dependency discovery for the
//! model viewer. Never infer an animation from a matching filename alone.
use evolve_core::{
    AnimationReference, AssetResolution, Inventory, LibraryReferenceStatus, animation_references,
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
    pub material_note: String,
    pub clips: Vec<AutoClip>,
    pub idle: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub warnings: Vec<String>,
}

/// Local entries have precedence only when the exact virtual path is unique.
/// A saved library hit must still be manifest-verified by the core resolver.
fn exact(
    inv: &Inventory,
    virtual_name: &str,
    imported: Option<&Path>,
    build: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    match resolve_virtual(inv, virtual_name) {
        AssetResolution::Found(relative) => {
            let root = std::fs::canonicalize(&inv.root).map_err(|e| e.to_string())?;
            let file = PathBuf::from(&inv.root).join(relative);
            let meta = std::fs::symlink_metadata(&file).map_err(|e| e.to_string())?;
            let canonical = std::fs::canonicalize(&file).map_err(|e| e.to_string())?;
            if !meta.file_type().is_file() || !canonical.starts_with(root) {
                return Err(format!("Unsafe indexed asset: {virtual_name}"));
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

pub fn discover(
    relative: &str,
    inv: &Inventory,
    imported: Option<&Path>,
    build: Option<&str>,
) -> AutoDiscovery {
    let mut found = AutoDiscovery::default();
    let canonical = relative.replace('\\', "/");
    let parent = canonical.rsplit_once('/').map_or("", |(p, _)| p);
    let bases = stems(&canonical);
    // Same-folder material names are an explicit fallback, not a general
    // basename search across characters or a random library texture.
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
    if found.material.is_none() && found.warnings.is_empty() {
        found.material_note = "No verified material for this mesh".into();
    }
    // A same-character CDF is allowed to identify a source skeleton only
    // when the selected model really is that CDF's skeleton or attachment.
    for base in &bases {
        let name = join(parent, base, "cdf");
        let Some(cdf_path) = (match exact(inv, &name, imported, build) {
            Ok(value) => value,
            Err(e) => {
                found.warnings.push(e);
                break;
            }
        }) else {
            continue;
        };
        let cdf = match evolve_core::read_cdf(&cdf_path) {
            Ok(value) => value,
            Err(e) => {
                found.warnings.push(format!("CDF metadata invalid: {e}"));
                break;
            }
        };
        let key = canonical.to_ascii_lowercase();
        let key_without_m = key
            .strip_suffix(".skinm")
            .map(|prefix| format!("{prefix}.skin"));
        let belongs = cdf.skeleton.eq_ignore_ascii_case(&canonical)
            || cdf.attachments.iter().any(|a| {
                a.binding.eq_ignore_ascii_case(&canonical)
                    || key_without_m
                        .as_ref()
                        .is_some_and(|p| a.binding.eq_ignore_ascii_case(p))
            });
        if !belongs {
            continue;
        }
        match exact(inv, &cdf.skeleton, imported, build) {
            Ok(Some(path)) => found.skeleton = Some(path),
            Ok(None) => found
                .warnings
                .push(format!("CDF skeleton not imported: {}", cdf.skeleton)),
            Err(e) => found.warnings.push(e),
        }
        break;
    }
    // Only this character's matching skeleton animation configuration.
    let mut configs = bases.clone();
    for base in &bases {
        if !base.to_ascii_lowercase().ends_with("_skeleton") {
            configs.push(format!("{base}_skeleton"));
        }
    }
    configs.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    for stem in configs {
        let name = join(parent, &stem, "chrparams");
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
    let mut preferred: Vec<(usize, (bool, bool, bool, String))> = Vec::new();
    for (i, r) in references.iter().enumerate() {
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
            preferred.push((i, rank));
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
            let r: &AnimationReference = &references[index];
            match exact(inv, &r.path, imported, build) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_core::scan_directory;

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
            <Animation name="swim_idle" path="swim_idle.caf"/>
            <Animation name="stand_idle_loop" path="stand_idle_loop.caf"/>
            <Animation name="walk_forward" path="walk_forward.caf"/>
        </AnimationList></Params>"##;
        std::fs::write(character.join("kala.chrparams"), catalog).unwrap();
        for clip in ["swim_idle.caf", "stand_idle_loop.caf", "walk_forward.caf"] {
            std::fs::write(animations.join(clip), b"fixture").unwrap();
        }
        let inventory = scan_directory(temp.path()).unwrap();
        let found = discover("characters/kala/kala.skin", &inventory, None, None);
        assert!(found.config.is_some());
        assert_eq!(found.clips.len(), 3);
        assert_eq!(found.clips[1].label, "stand_idle_loop");
        assert!(found.idle.unwrap().ends_with("stand_idle_loop.caf"));
        assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    }

    #[test]
    fn declared_clip_requires_exact_safe_name() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("selected.caf"), b"fixture").unwrap();
        let inventory = scan_directory(temp.path()).unwrap();
        assert!(resolve_declared(&inventory, "selected.caf", None, None).unwrap().is_some());
        assert!(resolve_declared(&inventory, "other.caf", None, None).unwrap().is_none());
        assert!(resolve_declared(&inventory, "../selected.caf", None, None).unwrap().is_none());
        assert!(resolve_declared(&inventory, "selected.dba", None, None).is_err());
    }

    #[test]
    fn idle_ranking_skips_contextual_motions() {
        assert!(idle_priority("stand_idle_loop").unwrap() < idle_priority("swim_idle").unwrap());
        assert_eq!(idle_priority("walk_forward"), None);
    }
}
