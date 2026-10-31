//! Read-only exact shared PAK inventory relevant to terrain and particle FX.
//! No proprietary asset bytes are embedded; these labels are the original
//! installed PAK names, not guesses based on user-supplied ZIP filenames.
use evolve_core::{ImportedPak, imported_archive_preview_inventory, imported_library};
use std::path::{Path, PathBuf};

const SHARED_EFFECT_PAKS: [&str; 4] = [
    "textures.pak",
    "materials_min_particle.pak",
    "objects_min_effects.pak",
    "objects_basic_effects.pak",
];

fn game_root(pak: &ImportedPak) -> Option<PathBuf> {
    Path::new(&pak.source)
        .ancestors()
        .find(|p| {
            p.file_name()
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("game"))
        })
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
}
fn count_asset(counts: &mut EffectCounts, original: &str) {
    let path = original.trim_start_matches("files/").replace('\\', "/").to_ascii_lowercase();
    if path.ends_with(".mtl") {
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
    let Some(game) = game_root(selected) else {
        return vec!["Shared textures/effects lookup unavailable: original map Game directory cannot be verified.".into()];
    };
    let Some(root) = Path::new(&selected.directory).parent() else {
        return vec!["Shared textures/effects lookup unavailable: imported Asset Library root missing.".into()];
    };
    let library = match imported_library(root) {
        Ok(catalog) => catalog,
        Err(error) => return vec![format!("Shared texture PAK lookup failed: {error}")],
    };
    let build = (selected.builds.len() == 1).then(|| selected.builds[0].as_str());
    let mut lines = Vec::new();
    for name in SHARED_EFFECT_PAKS {
        let matches = library.archives.iter().filter(|pak| {
            pak.name.eq_ignore_ascii_case(name)
                && game_root(pak).as_deref() == Some(game.as_path())
                && build.is_none_or(|label| {
                    pak.builds.is_empty() || pak.builds.iter().any(|other| other == label)
                })
                && (build.is_some() || pak.builds.len() <= 1)
        }).collect::<Vec<_>>();
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
                    "{name} verified (same install): {} terrain, {} particle, {} decals, {} roads DDS heads; {} other DDS heads; {} material definitions, {} effect meshes, {} particle library XML. Full high-resolution decoding still requires complete matching numbered mip fragments.",
                    c.terrain_dds_heads, c.particle_dds_heads, c.decal_dds_heads,
                    c.roads_dds_heads, c.other_dds_heads, c.material_definitions,
                    c.effect_geometry, c.particle_library_xml
                ));
            }
            Err(error) => lines.push(format!(
                "{name}: original imported source manifest unavailable ({error})"
            )),
        }
    }
    lines
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
    }
}
