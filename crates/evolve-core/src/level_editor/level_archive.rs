//! How the Asset Library and Level Editor present imported map archives.
//!
//! 2012-era CryEngine maps ship as `Game/Levels/<map>/level.pak` beside a
//! separate `Game/Levels/<map>/terraintexture.pak`. The texture container holds
//! baked terrain color tiles, never a loadable map, so it must not be offered
//! as a level. The level PAK itself has a generic filename, so lists label it
//! with its owner folder while keeping the source location visible.
use evolve_core::ImportedPak;

fn path_parts(path: &str) -> Vec<String> {
    path.replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn usable_folder(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".."
}

/// The map's baked terrain color container. The Asset Library classifies it
/// with Levels because of its name, but it is a ZIP of RGB24 tiles and can
/// never be opened as a map.
pub(super) fn is_terrain_texture_pak(pak: &ImportedPak) -> bool {
    pak.name.eq_ignore_ascii_case("terraintexture.pak")
}

/// The `Game/Levels/<map>` owner folder of an original 2012 map archive.
/// Only the exact map-owned `level.pak` layout has an authoritative owner;
/// any other `level.pak` keeps its filename.
pub(super) fn map_owner(pak: &ImportedPak) -> Option<String> {
    if !pak.name.eq_ignore_ascii_case("level.pak") {
        return None;
    }
    let parts = path_parts(&pak.source);
    let [.., game, levels, map, file] = parts.as_slice() else {
        return None;
    };
    if !file.eq_ignore_ascii_case("level.pak")
        || !levels.eq_ignore_ascii_case("levels")
        || !game.eq_ignore_ascii_case("game")
        || !usable_folder(map)
    {
        return None;
    }
    Some(map.clone())
}

/// Whether an imported archive should be listed as a level. TerrainTexture.pak
/// is always excluded; a generic `level.pak` is a map only when its source
/// proves its owner folder. Everything else keeps the filename/category
/// heuristic used for the shipped Levels archives.
pub(super) fn offers_as_map(pak: &ImportedPak) -> bool {
    if is_terrain_texture_pak(pak) {
        return false;
    }
    if pak.name.eq_ignore_ascii_case("level.pak") {
        return map_owner(pak).is_some();
    }
    crate::level_library::likely_level_pak(pak)
}

/// The list title: the owner folder for a 2012 map archive, otherwise the
/// imported filename.
pub(super) fn display_title(pak: &ImportedPak) -> String {
    map_owner(pak).unwrap_or_else(|| pak.name.clone())
}

/// Human-scale labels in the main table; original paths are always available
/// under Details. On Windows Path::parent is not reliable for C:\\ paths when
/// tests run on Linux, so normalize only for display and comparison.
pub(super) fn source_label(source: &str) -> String {
    let normalized = source.replace('\\', "/");
    let pieces = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if let Some(game) = pieces
        .iter()
        .rposition(|part| part.eq_ignore_ascii_case("game"))
        && game >= 1
    {
        let parent = pieces[game - 1];
        if parent.to_ascii_lowercase().starts_with("build_") && game >= 2 {
            return format!("{} / {parent}", pieces[game - 2]);
        }
        return parent.to_owned();
    }
    pieces
        .iter()
        .rev()
        .nth(1)
        .or_else(|| pieces.last())
        .copied()
        .unwrap_or("Unknown source")
        .to_owned()
}

/// The subtitle a map chooser card shows: the exact original source location
/// for a map-owned level.pak, otherwise its build labels. With several builds
/// carrying identically named maps, the owner name alone is not sufficient.
pub(super) fn display_subtitle(pak: &ImportedPak) -> String {
    if map_owner(pak).is_some() {
        pak.source.clone()
    } else {
        pak.builds.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pak(name: &str, source: &str, group: &str) -> ImportedPak {
        ImportedPak {
            id: "archive".into(),
            name: name.into(),
            source: source.into(),
            directory: "/library/archive".into(),
            group: group.into(),
            description: "Imported record".into(),
            entries: 1,
            extracted_bytes: 1,
            warnings: 0,
            builds: vec!["Stage 2".into()],
        }
    }

    #[test]
    fn map_owned_level_pak_is_labelled_by_its_owner_folder() {
        let pak = pak(
            "level.pak",
            "C:\\Games\\2012 Evolve\\build_3\\Game\\Levels\\Forest_Rapids\\level.pak",
            "Levels",
        );
        assert_eq!(map_owner(&pak).as_deref(), Some("Forest_Rapids"));
        assert_eq!(display_title(&pak), "Forest_Rapids");
        assert_eq!(
            display_subtitle(&pak),
            "C:\\Games\\2012 Evolve\\build_3\\Game\\Levels\\Forest_Rapids\\level.pak"
        );
        assert!(offers_as_map(&pak));
    }

    #[test]
    fn terrain_texture_pak_is_never_offered_as_a_level() {
        // The filename classifier alone puts this archive in Levels.
        let pak = pak(
            "terraintexture.pak",
            "C:\\Games\\2012 Evolve\\build_3\\Game\\Levels\\Forest_Rapids\\terraintexture.pak",
            "Levels",
        );
        assert!(is_terrain_texture_pak(&pak));
        assert!(!offers_as_map(&pak));
        assert_eq!(display_title(&pak), "terraintexture.pak");
    }

    #[test]
    fn level_pak_outside_the_map_owner_folder_keeps_its_filename() {
        let pak = pak("level.pak", "D:\\Imports\\level.pak", "Levels");
        assert_eq!(map_owner(&pak), None);
        assert_eq!(display_title(&pak), "level.pak");
        assert!(!offers_as_map(&pak));
    }

    #[test]
    fn ordinary_level_archives_keep_the_existing_discovery() {
        let pak = pak(
            "Levels_Tutorial.pak",
            "C:\\Games\\Stage2\\Game\\Levels_Tutorial.pak",
            "Levels",
        );
        assert_eq!(map_owner(&pak), None);
        assert_eq!(display_title(&pak), "Levels_Tutorial.pak");
        assert_eq!(display_subtitle(&pak), "Stage 2");
        assert!(offers_as_map(&pak));
    }

    #[test]
    fn source_labels_preserve_build_provenance() {
        assert_eq!(
            source_label("C:\\Games\\ModdedEvolve\\EvolveGame\\Game\\X.pak"),
            "EvolveGame"
        );
        assert_eq!(
            source_label("C:/Games/2012 Evolve/build_3/Game/X.pak"),
            "2012 Evolve / build_3"
        );
    }
}
