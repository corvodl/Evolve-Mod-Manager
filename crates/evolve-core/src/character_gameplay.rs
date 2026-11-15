//! Conservative gameplay discovery from authored member namespaces.
//! A display name is never used to guess an archive or a game-internal ID.
use quick_xml::{Reader, events::Event};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceKind {
    Ability,
    Item,
    EffectSet,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentReference {
    pub kind: ReferenceKind,
    pub name: String,
}

/// Only explicit sItem/sAbility actor parameters establish a relationship.
/// Filename similarity, blueprint text and generic actor inheritance do not.
pub fn actor_references(bytes: &[u8]) -> Result<Vec<DocumentReference>, String> {
    let xml = crate::cryxml::xml_bytes(bytes, crate::gameplay_values::LIMIT)
        .map_err(|error| error.to_string())?;
    crate::cryxml::encode_xml(&xml, crate::gameplay_values::LIMIT)
        .map_err(|error| error.to_string())?;
    let mut reader = Reader::from_reader(xml.as_ref());
    let mut references = Vec::new();
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(tag) | Event::Empty(tag) if tag.name().as_ref() == b"param" => {
                let mut name = None;
                let mut value = None;
                for attribute in tag.attributes() {
                    let attribute = attribute.map_err(|error| error.to_string())?;
                    if attribute.key.as_ref() == b"name" || attribute.key.as_ref() == b"value" {
                        let decoded = attribute
                            .decode_and_unescape_value(reader.decoder())
                            .map_err(|error| error.to_string())?
                            .into_owned();
                        if attribute.key.as_ref() == b"name" {
                            name = Some(decoded);
                        } else {
                            value = Some(decoded);
                        }
                    }
                }
                let kind = match name.as_deref() {
                    Some("sAbility") => ReferenceKind::Ability,
                    Some("sItem") => ReferenceKind::Item,
                    _ => continue,
                };
                for token in value.unwrap_or_default().split('|') {
                    if token.is_empty()
                        || token.len() > 120
                        || !token
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    {
                        continue;
                    }
                    if references.len() >= 10_000 {
                        return Err("Too many authored actor references".into());
                    }
                    references.push(DocumentReference {
                        kind,
                        name: token.to_owned(),
                    });
                }
            }
            Event::Eof => break,
            _ => (),
        }
    }
    references.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| (left.kind as u8).cmp(&(right.kind as u8)))
    });
    references.dedup();
    Ok(references)
}

/// An item can explicitly apply a shared effect set. Other strings in item
/// documents are not evidence of an effect-set dependency.
pub fn item_effect_set_references(bytes: &[u8]) -> Result<Vec<DocumentReference>, String> {
    let xml = crate::cryxml::xml_bytes(bytes, crate::gameplay_values::LIMIT)
        .map_err(|error| error.to_string())?;
    crate::cryxml::encode_xml(&xml, crate::gameplay_values::LIMIT)
        .map_err(|error| error.to_string())?;
    let mut reader = Reader::from_reader(xml.as_ref());
    let mut references = Vec::new();
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(tag) | Event::Empty(tag) => {
                for attribute in tag.attributes() {
                    let attribute = attribute.map_err(|error| error.to_string())?;
                    if attribute.key.as_ref() != b"autoApplyEffectSet" {
                        continue;
                    }
                    let value = attribute
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|error| error.to_string())?;
                    if value.is_empty()
                        || value.len() > 120
                        || !value
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    {
                        continue;
                    }
                    if references.len() >= 10_000 {
                        return Err("Too many authored effect-set references".into());
                    }
                    references.push(DocumentReference {
                        kind: ReferenceKind::EffectSet,
                        name: value.into_owned(),
                    });
                }
            }
            Event::Eof => break,
            _ => (),
        }
    }
    references.sort_by(|left, right| left.name.cmp(&right.name));
    references.dedup();
    Ok(references)
}

pub fn referenced_document(reference: &DocumentReference, path: &str) -> bool {
    if !crate::mod_safety::gameplay_document(path) {
        return false;
    }
    let path = path.to_ascii_lowercase();
    let parts: Vec<_> = path.split('/').collect();
    let name = reference.name.to_ascii_lowercase();
    let Some(filename) = parts.last() else {
        return false;
    };
    if filename.strip_suffix(".xml") != Some(name.as_str()) {
        return false;
    }
    matches!(
        (reference.kind, parts.as_slice()),
        (ReferenceKind::Ability, ["libs", "abilities", ..])
            | (
                ReferenceKind::Ability,
                ["scripts", "entities", "abilities", "xml", ..]
            )
            | (ReferenceKind::Item, ["libs", "items", ..])
            | (
                ReferenceKind::Item,
                ["scripts", "entities", "items", "xml", ..]
            )
            | (ReferenceKind::EffectSet, ["libs", "effectsets", ..])
    )
}

/// Resolve only unique source-qualified members. Two PAKs with the same
/// authored member path are still distinct candidates and remain ambiguous.
pub fn unique_referenced_members(
    references: &[DocumentReference],
    candidates: &[(String, String)],
) -> Vec<usize> {
    let mut found = std::collections::BTreeSet::new();
    for reference in references {
        let mut matches = candidates
            .iter()
            .enumerate()
            .filter(|(_, (_, path))| referenced_document(reference, path));
        if let Some((index, _)) = matches.next()
            && matches.next().is_none()
        {
            found.insert(index);
        }
    }
    found.into_iter().collect()
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
    fn actor_links_require_explicit_typed_parameters_and_exact_document_names() {
        let xml = br#"<Actor base="BaseMerc"><param name="sBlueprint" value="Maggie"/><param name="sItem" value="MobileArenaTriggerWeapon|SubMachineGunA"/><param name="sAbility" value="RockyDome1"/><param name="sItem" value="../foreign"/></Actor>"#;
        let links = actor_references(xml).unwrap();
        assert_eq!(links.len(), 3);
        let item = links
            .iter()
            .find(|link| link.name == "SubMachineGunA")
            .unwrap();
        assert!(referenced_document(
            item,
            "libs/Items/Mercs/Maggie/SubMachineGunA.xml"
        ));
        assert!(!referenced_document(
            item,
            "libs/Items/Mercs/Maggie/SubMachineGunA_extra.xml"
        ));
        assert!(!referenced_document(
            item,
            "libs/Abilities/Mercs/SubMachineGunA.xml"
        ));
        let ability = links.iter().find(|link| link.name == "RockyDome1").unwrap();
        assert!(referenced_document(
            ability,
            "Scripts/Entities/Abilities/XML/Rocky/RockyDome1.xml"
        ));
        assert!(!referenced_document(
            ability,
            "libs/Items/Rocky/RockyDome1.xml"
        ));
        assert!(actor_references(b"<!DOCTYPE foo><Actor/>").is_err());
        let candidates = vec![
            (
                "stage-two".into(),
                "libs/Items/Shared/SubMachineGunA.xml".into(),
            ),
            ("2012".into(), "libs/Items/Shared/SubMachineGunA.xml".into()),
            (
                "stage-two".into(),
                "libs/Abilities/Shared/RockyDome1.xml".into(),
            ),
        ];
        assert_eq!(unique_referenced_members(&links, &candidates), vec![2]);
        assert_eq!(unique_referenced_members(&links, &candidates[..1]), vec![0]);
    }

    #[test]
    fn item_effect_sets_are_only_linked_by_authored_attribute() {
        let item = br#"<item name="MercHealthRegen"><feature autoApplyEffectSet="MercHealthRegen" other="Unrelated"/></item>"#;
        let refs = item_effect_set_references(item).unwrap();
        assert_eq!(refs.len(), 1);
        assert!(referenced_document(
            &refs[0],
            "libs/EffectSets/MercHealthRegen.xml"
        ));
        assert!(!referenced_document(
            &refs[0],
            "libs/Items/Mercs/Shared/MercHealthRegen.xml"
        ));
        assert!(item_effect_set_references(b"<!DOCTYPE foo><item/>").is_err());
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
