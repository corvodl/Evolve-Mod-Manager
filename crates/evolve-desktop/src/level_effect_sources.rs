//! Read-only exact shared PAK inventory relevant to terrain and particle FX.
//! No proprietary asset bytes are embedded; these labels are the original
//! installed PAK names, not guesses based on user-supplied ZIP filenames.
use evolve_core::{ImportedPak, imported_archive_preview_inventory, imported_library};
use std::path::{Path, PathBuf};

const SHARED_EFFECT_PAKS: [&str; 8] = [
    "textures.pak",
    "materials_min_particle.pak",
    "objects_min_effects.pak",
    "objects_basic_effects.pak",
    "shaders.pak",
    "shadersbin.pak",
    "shadercache.pak",
    "engine.pak",
];

fn installation_root(pak: &ImportedPak) -> Option<PathBuf> {
    // Evolve's shared assets can be in Game/ while its D3D11 shaders and
    // graphics configuration can be in Engine/. Compare the common parent
    // of exactly these source roots, never unrelated installs or basenames.
    Path::new(&pak.source)
        .ancestors()
        .find(|p| {
            p.file_name().is_some_and(|name| {
                name.to_string_lossy().eq_ignore_ascii_case("game")
                    || name.to_string_lossy().eq_ignore_ascii_case("engine")
            })
        })
        .and_then(|p| p.parent())
        .and_then(|p| std::fs::canonicalize(p).ok())
}

#[derive(Default, Debug, PartialEq, Eq)]
struct EffectCounts {
    terrain_dds_heads: usize,
    particle_dds_heads: usize,
    decal_dds_heads: usize,
    roads_dds_heads: usize,
    other_dds_heads: usize,
    material_definitions: usize,
    effect_geometry: usize,
    particle_library_xml: usize,
    shader_extensions: usize,
    compiled_shader_metadata: usize,
    compiled_shader_variants: usize,
    engine_configurations: usize,
}
fn count_asset(counts: &mut EffectCounts, original: &str) {
    let path = original
        .trim_start_matches("files/")
        .replace('\\', "/")
        .to_ascii_lowercase();
    if path.starts_with("shaders/") && path.ends_with(".ext") {
        counts.shader_extensions += 1;
    } else if path.starts_with("shaders/cache/")
        && (path.ends_with(".cfxb") || path.ends_with(".cfib"))
    {
        counts.compiled_shader_metadata += 1;
    } else if path.starts_with("shaders/cache/") && path.ends_with(".fxcb") {
        counts.compiled_shader_variants += 1;
    } else if path.starts_with("config/") && path.ends_with(".cfg") {
        counts.engine_configurations += 1;
    } else if path.ends_with(".mtl") {
        counts.material_definitions += 1;
    } else if path.ends_with(".cgf") {
        counts.effect_geometry += 1;
    } else if path.ends_with(".xml")
        && (path.starts_with("libs/particles/") || path.contains("/libs/particles/"))
    {
        counts.particle_library_xml += 1;
    } else if path.ends_with(".dds.0") {
        if path.starts_with("textures/terrain/") {
            counts.terrain_dds_heads += 1;
        } else if path.starts_with("textures/particle/") {
            counts.particle_dds_heads += 1;
        } else if path.starts_with("textures/decals/") {
            counts.decal_dds_heads += 1;
        } else if path.starts_with("textures/roads/") {
            counts.roads_dds_heads += 1;
        } else {
            counts.other_dds_heads += 1;
        }
    }
}
pub(super) fn shared_effect_sources(selected: &ImportedPak) -> Vec<String> {
    let Some(game) = installation_root(selected) else {
        return vec!["Shared textures/effects lookup unavailable: original map Game directory cannot be verified.".into()];
    };
    let Some(root) = Path::new(&selected.directory).parent() else {
        return vec![
            "Shared textures/effects lookup unavailable: imported Asset Library root missing."
                .into(),
        ];
    };
    let library = match imported_library(root) {
        Ok(catalog) => catalog,
        Err(error) => return vec![format!("Shared texture/shader PAK lookup failed: {error}")],
    };
    let build = (selected.builds.len() == 1).then(|| selected.builds[0].as_str());
    let mut lines = Vec::new();
    for name in SHARED_EFFECT_PAKS {
        let matches = library
            .archives
            .iter()
            .filter(|pak| {
                pak.name.eq_ignore_ascii_case(name)
                    && installation_root(pak).as_deref() == Some(game.as_path())
                    && build.is_none_or(|label| {
                        pak.builds.is_empty() || pak.builds.iter().any(|other| other == label)
                    })
                    && (build.is_some() || pak.builds.len() <= 1)
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            lines.push(format!(
                "{name}: {} exact same-install source matches; no cross-build substitution. {}",
                matches.len(),
                if matches.is_empty() {
                    "Import this PAK from the selected Game build to resolve its assets."
                } else {
                    "Source PAK is ambiguous; cannot safely choose its textures."
                }
            ));
            continue;
        }
        match imported_archive_preview_inventory(matches[0]) {
            Ok(inventory) => {
                let mut c = EffectCounts::default();
                for entry in inventory.entries {
                    count_asset(&mut c, &entry.relative_path);
                }
                lines.push(format!(
                    "{name} verified (same installation/build): {} terrain, {} particle, {} decal, {} road DDS heads; {} other DDS heads; {} materials, {} effect meshes, {} particle library XML; {} shader .ext feature declarations, {} compiled .cfxb/.cfib metadata, {} D3D11 .fxcb variants, {} engine .cfg files. Shader binaries are reference data, not reusable native wgpu shaders.",
                    c.terrain_dds_heads, c.particle_dds_heads, c.decal_dds_heads,
                    c.roads_dds_heads, c.other_dds_heads, c.material_definitions,
                    c.effect_geometry, c.particle_library_xml,
                    c.shader_extensions, c.compiled_shader_metadata,
                    c.compiled_shader_variants, c.engine_configurations
                ));
            }
            Err(error) => lines.push(format!(
                "{name}: original imported source manifest unavailable ({error})"
            )),
        }
    }
    lines
}

/// Resolve only map-requested particle libraries, by original exact
/// namespace, from the SAME installation and uniquely selected build.
/// This is an inventory/decoder step, not yet a live GPU particle renderer.
pub(super) fn verify_source_particle_placements(
    selected: &ImportedPak,
    placed: &[crate::level_atmosphere::ParticlePlacement],
) -> Vec<String> {
    if placed.is_empty() { return Vec::new(); }
    let Some(install) = installation_root(selected) else {
        return vec!["Particle library matching blocked: original Game installation unverified".into()];
    };
    let Some(library_root) = Path::new(&selected.directory).parent() else {
        return vec!["Particle library matching blocked: imported Asset Library root missing".into()];
    };
    let Ok(catalog) = imported_library(library_root) else {
        return vec!["Particle library matching blocked: Asset Library metadata unreadable".into()];
    };
    let build = (selected.builds.len()==1).then(|| selected.builds[0].as_str());
    let matches = catalog.archives.iter().filter(|pak|
        pak.name.eq_ignore_ascii_case("libs.pak")
            && installation_root(pak).as_deref()==Some(install.as_path())
            && build.is_none_or(|label|pak.builds.is_empty()
                || pak.builds.iter().any(|p|p==label))
            && (build.is_some() || pak.builds.len()<=1)
    ).collect::<Vec<_>>();
    if matches.len()!=1 {
        return vec![format!(
            "Particle libraries: {} same-install libs.pak imports (require exactly 1); no effect alias or cross-build fallback",
            matches.len()
        )];
    }
    let Ok(inv)=imported_archive_preview_inventory(matches[0]) else {
        return vec!["Original libs.pak imported manifest is invalid or unreadable".into()];
    };
    let root=PathBuf::from(inv.root);
    let requests=placed.iter().filter_map(|p|p.effect.split_once('.').map(|(lib,_)|lib.to_ascii_lowercase()))
        .collect::<std::collections::BTreeSet<_>>();
    let mut found=std::collections::HashMap::<String,Vec<evolve_core::ParticleEmitter>>::new();
    let mut warnings=Vec::new();
    for lib in requests.into_iter().take(32) {
        // Do not join an authored untrusted XML value into a filesystem path.
        // Search only already validated import manifest virtual names.
        let path=format!("libs/particles/{lib}.xml");
        let candidates=inv.entries.iter().filter(|entry|
            entry.relative_path.replace('\\',"/").eq_ignore_ascii_case(&path)
                && entry.byte_size<=32*1024*1024
        ).collect::<Vec<_>>();
        if candidates.len()!=1 {
            warnings.push(format!("{lib}: exact source particle XML missing or ambiguous"));
            continue;
        }
        let source=root.join(&candidates[0].relative_path);
        let verified=std::fs::canonicalize(&source).ok()
            .filter(|resolved| resolved.starts_with(&root))
            .filter(|resolved| {
                std::fs::metadata(resolved).ok()
                    .is_some_and(|m|m.len()==candidates[0].byte_size)
            });
        let Some(source)=verified else {
            warnings.push(format!("{lib}: particle library bytes missing/modified"));
            continue;
        };
        match std::fs::read(source)
            .map_err(|e|e.to_string())
            .and_then(|bytes|evolve_core::decode_particle_library(&bytes))
        {
            Ok(emitters) => {found.insert(lib,emitters);}
            Err(e) => warnings.push(format!("{lib}: original CryXmlB emitter decode failed: {e}")),
        }
    }
    let mut matched=0usize;
    let mut missing=0usize;
    let mut child_definitions=0usize;
    for place in placed.iter().take(2048) {
        let Some((library,effect))=place.effect.split_once('.') else {
            missing+=1;
            continue;
        };
        let Some(emitters)=found.get(&library.to_ascii_lowercase()) else {
            missing+=1;
            continue;
        };
        if emitters.iter().any(|e|e.effect.eq_ignore_ascii_case(effect)) {
            matched+=1;
            child_definitions+=emitters.iter().filter(|e|
                e.effect.len()>effect.len()
                    && e.effect[..effect.len()].eq_ignore_ascii_case(effect)
                    && e.effect.as_bytes().get(effect.len())==Some(&b'.')
            ).count();
        } else {
            missing+=1;
        }
    }
    let mut out=vec![format!(
        "Exact same-build libs.pak particle references: {matched}/{} map placements resolve to original ParticleLibrary entries; {} nested effect definitions; {missing} unresolved (not substituted). CPU simulation source parser available; sprite rendering still pending.",
        placed.len(),child_definitions
    )];
    out.extend(warnings.into_iter().take(8));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_pak_labels_prevent_conflating_min_and_basic_archives() {
        assert_eq!(
            SHARED_EFFECT_PAKS,
            [
                "textures.pak",
                "materials_min_particle.pak",
                "objects_min_effects.pak",
                "objects_basic_effects.pak",
                "shaders.pak",
                "shadersbin.pak",
                "shadercache.pak",
                "engine.pak",
            ]
        );
    }
    #[test]
    fn shared_dds_categories_match_authored_virtual_paths() {
        let mut c = EffectCounts::default();
        for p in [
            "files/textures/terrain/arctic/ice_a_terr.dds.0",
            "textures/particle/light/glow_prism_01a.dds.0",
            "textures/decals/arctic/ice_decal_chunks_a.dds.0",
            "textures/roads/asphalt.dds.0",
            "materials/particle/smoke.mtl",
            "objects/effects/light_beam.cgf",
            "libs/particles/smoke.xml",
            "Shaders/Particles.ext",
            "Shaders/Cache/D3D11/Particles.cfxb",
            "Shaders/Cache/D3D11/CGPShaders/Particles@ParticlePS.fxcb",
            "Config/CVarGroups/sys_spec_Particles.cfg",
            "textures/terrain/arctic/ice_a_terr.dds.1",
        ] {
            count_asset(&mut c, p);
        }
        assert_eq!(c.terrain_dds_heads, 1);
        assert_eq!(c.particle_dds_heads, 1);
        assert_eq!(c.decal_dds_heads, 1);
        assert_eq!(c.roads_dds_heads, 1);
        assert_eq!(c.material_definitions, 1);
        assert_eq!(c.effect_geometry, 1);
        assert_eq!(c.particle_library_xml, 1);
        assert_eq!(c.shader_extensions, 1);
        assert_eq!(c.compiled_shader_metadata, 1);
        assert_eq!(c.compiled_shader_variants, 1);
        assert_eq!(c.engine_configurations, 1);
    }
}
