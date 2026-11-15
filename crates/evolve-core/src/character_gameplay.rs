//! Conservative gameplay discovery from authored member namespaces.
//! A display name is never used to guess an archive or a game-internal ID.

/// In-game name and internal actor namespace confirmed against imported
/// actor and item manifests. This is discovery metadata, not permission to edit.
pub const CHARACTERS: &[(&str, &str)] = &[
    ("Goliath", "Goliath"),
    ("Meteor Goliath", "GoliathMeteor"),
    ("Kraken", "Grounder"),
    ("Elder Kraken", "KrakenElder"),
    ("Wraith", "Teleporter"),
    ("Behemoth", "Rocky"),
    ("Glacial Behemoth", "RockyBobcicle"),
    ("Gorgon", "Lurker"),
    ("Markov", "Markov"),
    ("Blitz Markov", "MarkovBlitz"),
    ("Hyde", "Hyde"),
    ("Parnell", "Parnell"),
    ("Torvald", "Torvald"),
    ("Lennox", "Lennox"),
    ("Val", "Val"),
    ("Rogue Val", "ValRogue"),
    ("Lazarus", "Lazarus"),
    ("Caira", "Caira"),
    ("Quantum Caira", "CairaLowgrav"),
    ("Slim", "Slim"),
    ("Emet", "Emet"),
    ("Paladin Parnell", "ParnellTank"),
    ("Hank", "Hank"),
    ("Tech Sergeant Hank", "HankSgt"),
    ("Bucket", "Bucket"),
    ("Cabot", "Cabot"),
    ("Battle Cabot", "CabotBattle"),
    ("Sunny", "Sunny"),
    ("Kala", "Tech"),
    ("Maggie", "Maggie"),
    ("Wasteland Maggie", "MaggieSavage"),
    ("Griffin", "Griffin"),
    ("Electro Griffin", "GriffinElectro"),
    ("Abe", "Abe"),
    ("Renegade Abe", "AbeRenegade"),
    ("Crow", "Crow"),
    ("Jack", "Scrap"),
];

/// Creature IDs with direct authored actor documents in the Stage 2 import.
/// Non-wildlife actors (pets, drones, civilians and tutorial AI) stay out.
pub const WILDLIFE: &[(&str, &str)] = &[
    ("Armadon Bull", "ArmadonBull"),
    ("Blitz Leopard", "BlitzLeopard"),
    ("Canyon Eel", "CanyonEel"),
    ("Canyon Strider", "CanyonStrider"),
    ("Cephaladon", "Cephaladon"),
    ("Crowbill Sloth", "CrowbillSloth"),
    ("Dune Beetle", "DuneBeetle"),
    ("Glaciopod", "Glaciopod"),
    ("Mammoth Bird", "MammothBird"),
    ("Marsh Strider", "MarshStrider"),
    ("Megamouth", "Megamouth"),
    ("Nomad", "Nomad"),
    ("Obsidian Grub", "ObsidianGrub"),
    ("Reaver", "Reaver"),
    ("Spotter", "Spotter"),
    ("Steamadon", "Steamadon"),
    ("Trapjaw", "Trapjaw"),
    ("Tyrant", "Tyrant"),
    ("Venom Hound", "VenomHound"),
];

pub fn owns_wildlife_document(name: &str, path: &str) -> bool {
    let Some((_, id)) = WILDLIFE.iter().find(|(label, _)| *label == name) else {
        return false;
    };
    let lower = path.to_ascii_lowercase();
    let Some(filename) = lower.strip_prefix("libs/actor/creatures/") else {
        return false;
    };
    let id = id.to_ascii_lowercase();
    let stem = filename.strip_suffix(".actorparams").unwrap_or("");
    // Avoid attributing a pet or another species with a shared prefix.
    stem == id
        || ["elite", "rabid", "protected"]
            .iter()
            .any(|variant| stem == format!("{id}{variant}"))
}

pub fn internal_id(display_name: &str) -> Option<&'static str> {
    CHARACTERS
        .iter()
        .find(|(name, _)| *name == display_name)
        .map(|(_, id)| *id)
}

/// Match only an authored per-character namespace. Shared actor bases and
/// global effect sets are not silently attributed to a playable character.
pub fn owns_document(display_name: &str, path: &str) -> bool {
    if !crate::mod_safety::gameplay_document(path) {
        return false;
    }
    let Some(id) = internal_id(display_name) else {
        return false;
    };
    let lower = path.to_ascii_lowercase();
    let parts: Vec<_> = lower.split('/').collect();
    let id = id.to_ascii_lowercase();
    match parts.as_slice() {
        ["libs", "actor", "mercs", filename] => {
            let stem = filename.strip_suffix(".actorparams").unwrap_or("");
            stem == id
        }
        ["libs", "actor", "monsters", filename] => {
            let stem = filename.strip_suffix(".actorparams").unwrap_or("");
            stem == id
                || ["a", "coop", "bghcoop"]
                    .iter()
                    .any(|suffix| stem == format!("{id}{suffix}"))
        }
        ["libs", "abilities", owner, ..] if *owner == id => true,
        ["libs", "items", "mercs" | "monsters", owner, ..] if *owner == id => true,
        ["scripts", "entities", "abilities", "xml", owner, ..] if *owner == id => true,
        [
            "scripts",
            "entities",
            "items",
            "xml",
            "weapons" | "ammo",
            owner,
            ..,
        ] if *owner == id => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_authored_namespaces_not_cosmetic_names_or_neighbor_variants() {
        assert_eq!(CHARACTERS.len(), 37);
        assert!(owns_document(
            "Behemoth",
            "libs/Actor/Monsters/RockyA.actorparams"
        ));
        assert!(owns_document(
            "Kraken",
            "libs/Abilities/Grounder/Lightning.xml"
        ));
        assert!(owns_document(
            "Markov",
            "libs/Items/Mercs/Markov/LightningGun.xml"
        ));
        assert!(!owns_document(
            "Goliath",
            "libs/Abilities/GoliathMeteor/Rock.xml"
        ));
        assert!(!owns_document(
            "Markov",
            "libs/Actor/Mercs/MarkovBlitz.actorparams"
        ));
        assert!(!owns_document(
            "Kala",
            "libs/Actor/Mercs/TechSgt.actorparams"
        ));
        assert!(!owns_document(
            "Goliath",
            "libs/Actor/Monsters/BaseMonster.actorparams"
        ));
        assert!(!owns_document("Goliath", "objects/goliath.xml"));
        assert!(owns_document(
            "Behemoth",
            "Scripts/Entities/Abilities/XML/Rocky/RockyDome1.xml"
        ));
        assert!(!owns_document(
            "Glacial Behemoth",
            "Scripts/Entities/Abilities/XML/Rocky/RockyDome1.xml"
        ));
        assert!(owns_wildlife_document(
            "Trapjaw",
            "libs/Actor/Creatures/Trapjaw.actorparams"
        ));
        assert!(!owns_wildlife_document(
            "Trapjaw",
            "libs/Actor/Creatures/TrapjawPet.actorparams"
        ));
        assert!(owns_wildlife_document(
            "Armadon Bull",
            "libs/Actor/Creatures/ArmadonBullElite.actorparams"
        ));
    }

    #[test]
    #[ignore = "requires EVOLVE_BENCH_LIBRARY pointing to a local imported Asset Library"]
    fn local_imported_roster_has_direct_gameplay_documents() {
        let root = std::path::PathBuf::from(std::env::var_os("EVOLVE_BENCH_LIBRARY").unwrap());
        let catalog = crate::library_catalog::imported_library(&root).unwrap();
        let mut paths = Vec::new();
        for archive in &catalog.archives {
            if archive.name.eq_ignore_ascii_case("libs.pak") || archive.name == "Scripts.pak" {
                let inventory =
                    crate::library_catalog::imported_archive_preview_inventory(archive).unwrap();
                paths.extend(
                    inventory
                        .entries
                        .into_iter()
                        .map(|entry| entry.relative_path),
                );
            }
        }
        for (name, _) in CHARACTERS {
            assert!(
                paths.iter().any(|path| owns_document(name, path)),
                "No directly authored gameplay document for {name}"
            );
        }
    }
}
