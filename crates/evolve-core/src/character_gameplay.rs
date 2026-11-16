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
    wildlife_stem(id, filename.strip_suffix(".actorparams").unwrap_or(""))
}

/// Avoid attributing a pet or another species with a shared prefix.
fn wildlife_stem(id: &str, stem: &str) -> bool {
    let id = id.to_ascii_lowercase();
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

/// A roster entry: a playable character or a wildlife species.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RosterEntry {
    Character(&'static str),
    Wildlife(&'static str),
}

enum ScriptFile {
    /// `subtypes_<id>.lua`: numbered stages, with variants named by their ID.
    Monster(&'static str),
    /// `subtype_<id>.lua`: one class global, variants as `<Class>_<Suffix>`.
    Hunter(&'static str),
    /// `Scripts/Entities/AI/<Species>[Elite|Rabid|Protected]_x.lua`.
    Creature(&'static str),
}

fn script_file(path: &str) -> Option<ScriptFile> {
    let lower = path.to_ascii_lowercase();
    let filename = lower.rsplit('/').next()?.strip_suffix(".lua")?;
    let roster_id = |id: &str| {
        CHARACTERS
            .iter()
            .find(|(_, candidate)| candidate.eq_ignore_ascii_case(id))
            .map(|(name, _)| *name)
    };
    if lower.starts_with("scripts/entities/actor/") {
        if let Some(id) = filename.strip_prefix("subtypes_") {
            return roster_id(id).map(ScriptFile::Monster);
        }
        return roster_id(filename.strip_prefix("subtype_")?).map(ScriptFile::Hunter);
    }
    let stem = lower
        .strip_prefix("scripts/entities/ai/")?
        .strip_suffix("_x.lua")?;
    WILDLIFE
        .iter()
        .find(|(_, id)| wildlife_stem(id, stem))
        .map(|(name, _)| ScriptFile::Creature(name))
}

/// Legacy actor or creature script that can define a roster entry.
pub fn roster_script(path: &str) -> bool {
    script_file(path).is_some()
}

/// Any actor script (including shared bases such as `player.lua`) or roster
/// creature script, as a read-only source for inheritance and preview.
pub fn actor_script(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    (lower.starts_with("scripts/entities/actor/") && lower.ends_with(".lua")) || roster_script(path)
}

/// The roster entry a global in a Legacy script defines. Global names repeat
/// across files (`SupportF` is both Sunny and Kala), so a global is only ever
/// interpreted inside the file that assigns it. Shared bases and minions
/// (`GoliathBase`, `GoliathMinion1`) belong to no entry.
pub fn script_global_owner(path: &str, global: &str) -> Option<RosterEntry> {
    let global = global.to_ascii_lowercase();
    let by_id = |id: &str| {
        CHARACTERS
            .iter()
            .find(|(_, candidate)| candidate.eq_ignore_ascii_case(id))
            .map(|(name, _)| RosterEntry::Character(name))
    };
    match script_file(path)? {
        ScriptFile::Monster(name) => {
            let stem = global.trim_end_matches(|c: char| c.is_ascii_digit());
            let id = internal_id(name)?.to_ascii_lowercase();
            if stem.len() == global.len() {
                None
            } else if stem == id || stem == format!("{id}a") {
                Some(RosterEntry::Character(name))
            } else {
                by_id(stem)
            }
        }
        ScriptFile::Hunter(name) => match global.split_once('_') {
            Some((_, suffix)) => by_id(&format!("{}{suffix}", internal_id(name)?)),
            None => Some(RosterEntry::Character(name)),
        },
        ScriptFile::Creature(name) => {
            let filename = path.rsplit('/').next()?.strip_suffix(".lua")?;
            filename
                .eq_ignore_ascii_case(&global)
                .then_some(RosterEntry::Wildlife(name))
        }
    }
}

/// Explicit references in a decoded subtype table: its `loadout` abilities,
/// `gameParams.inventory.items.<slot>.name` items and `gameParams.effectSets`.
pub fn script_references(table: &crate::lua51::Table) -> Vec<DocumentReference> {
    use crate::lua51::Value;
    let mut references = Vec::new();
    let mut push = |kind, value: &Value| {
        if let Value::Str(name) = value
            && !name.is_empty()
            && name.len() <= 120
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            references.push(DocumentReference {
                kind,
                name: name.clone(),
            });
        }
    };
    for ability in table.table("loadout").map_or(&[][..], |list| &list.items) {
        push(ReferenceKind::Ability, ability);
    }
    if let Some(game) = table.table("gameParams") {
        for set in game.table("effectSets").map_or(&[][..], |list| &list.items) {
            push(ReferenceKind::EffectSet, set);
        }
        for (_, slot) in game
            .table("inventory")
            .and_then(|inventory| inventory.table("items"))
            .map_or(&[][..], |items| &items.fields)
        {
            if let Value::Table(slot) = slot
                && let Some(item) = slot.get("name")
            {
                push(ReferenceKind::Item, item);
            }
        }
    }
    references.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| (left.kind as u8).cmp(&(right.kind as u8)))
    });
    references.dedup();
    references
}

/// `Properties.fileModel` of a decoded subtype table, if it names a CDF.
pub fn script_model(table: &crate::lua51::Table) -> Result<Option<String>, String> {
    let Some(model) = table
        .table("Properties")
        .and_then(|properties| properties.str("fileModel"))
    else {
        return Ok(None);
    };
    if !model.to_ascii_lowercase().ends_with(".cdf") {
        return Ok(None);
    }
    crate::archive::safe_member_name(&model.replace('\\', "/"))
        .map(Some)
        .map_err(|error| error.to_string())
}

/// Read literal `fileModel` assignments in a verified 2012 subtype document.
/// Never run Lua or scan compiled bytecode for strings: only the authored
/// assignment itself proves a model link.
pub fn authored_subtype_models(bytes: &[u8]) -> Result<Vec<String>, String> {
    if bytes.len() > 256 * 1024 || bytes.contains(&0) {
        return Err("Subtype source is not bounded plain text".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let mut models = std::collections::BTreeSet::new();
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        if key.trim() != "fileModel" {
            continue;
        }
        let value = value.trim();
        let quote = value.chars().next().unwrap_or_default();
        if quote != '"' && quote != '\'' {
            continue;
        }
        let Some((path, rest)) = value[1..].split_once(quote) else {
            continue;
        };
        if !rest.trim().trim_start_matches(',').trim().is_empty()
            || !path.to_ascii_lowercase().ends_with(".cdf")
        {
            continue;
        }
        let path = crate::archive::safe_member_name(&path.replace('\\', "/"))
            .map_err(|error| error.to_string())?;
        if models.len() >= 256 {
            return Err("Too many authored subtype models".into());
        }
        models.insert(path);
    }
    Ok(models.into_iter().collect())
}

/// Material references declared by a CDF, not inferred from a skeleton's or
/// mesh's filename. CryEngine permits extensionless material attributes.
pub fn authored_cdf_materials(bytes: &[u8]) -> Result<Vec<String>, String> {
    let cdf = crate::cdf::parse_cdf(bytes).map_err(|error| error.to_string())?;
    let xml =
        crate::cryxml::xml_bytes(bytes, 2 * 1024 * 1024).map_err(|error| error.to_string())?;
    let mut reader = Reader::from_reader(xml.as_ref());
    let mut materials = Vec::new();
    for attachment in cdf.attachments {
        if let Some(material) = attachment.material {
            materials.push(material);
        }
    }
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(tag) | Event::Empty(tag)
                if tag.name().as_ref().eq_ignore_ascii_case(b"Model") =>
            {
                for attribute in tag.attributes() {
                    let attribute = attribute.map_err(|error| error.to_string())?;
                    if !attribute.key.as_ref().eq_ignore_ascii_case(b"Material") {
                        continue;
                    }
                    materials.push(
                        attribute
                            .decode_and_unescape_value(reader.decoder())
                            .map_err(|error| error.to_string())?
                            .into_owned(),
                    );
                }
            }
            Event::Eof => break,
            _ => (),
        }
    }
    let mut paths = std::collections::BTreeSet::new();
    for material in materials {
        let material = material.replace('\\', "/");
        if material.is_empty() {
            continue;
        }
        let candidate = if material.to_ascii_lowercase().ends_with(".mtl") {
            material
        } else {
            format!("{material}.mtl")
        };
        paths.insert(
            crate::archive::safe_member_name(&candidate).map_err(|error| error.to_string())?,
        );
    }
    Ok(paths.into_iter().collect())
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
        ["libs", "abilities", folder, ..]
        | ["libs", "items", "mercs" | "monsters", folder, ..]
        | ["scripts", "entities", "abilities", "xml", folder, ..]
        | [
            "scripts",
            "entities",
            "items",
            "xml",
            "weapons" | "ammo",
            folder,
            ..,
        ] => parts
            .last()
            .is_some_and(|filename| folder_owner(folder, filename) == id),
        _ => false,
    }
}

/// Legacy keeps a variant's documents in its base character's folder, named
/// with the variant's internal ID (`Libs/Abilities/Goliath/GoliathMeteorCharge1.xml`).
/// Such a file belongs to that roster variant, never to the base character.
fn folder_owner(folder: &str, filename: &str) -> String {
    CHARACTERS
        .iter()
        .map(|(_, id)| id.to_ascii_lowercase())
        .filter(|variant| {
            variant.len() > folder.len()
                && variant.starts_with(folder)
                && filename.starts_with(variant.as_str())
        })
        .max_by_key(String::len)
        .unwrap_or_else(|| folder.to_owned())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceKind {
    Ability,
    Item,
    EffectSet,
    /// The root actor's authored `base`; its parameters are inherited.
    ActorBase,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentReference {
    pub kind: ReferenceKind,
    pub name: String,
}

fn push_tokens(
    references: &mut Vec<DocumentReference>,
    kind: ReferenceKind,
    value: &str,
) -> Result<(), String> {
    for token in value.split('|') {
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
    Ok(())
}

/// Only explicit sItem/sAbility actor parameters and the root actor's `base`
/// establish a relationship. Filename similarity and blueprint text do not.
pub fn actor_references(bytes: &[u8]) -> Result<Vec<DocumentReference>, String> {
    let xml = crate::cryxml::xml_bytes(bytes, crate::gameplay_values::LIMIT)
        .map_err(|error| error.to_string())?;
    crate::cryxml::encode_xml(&xml, crate::gameplay_values::LIMIT)
        .map_err(|error| error.to_string())?;
    let mut reader = Reader::from_reader(xml.as_ref());
    let mut references = Vec::new();
    let mut root_seen = false;
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(tag) | Event::Empty(tag) if !root_seen => {
                root_seen = true;
                if tag.name().as_ref() != b"Actor" {
                    continue;
                }
                for attribute in tag.attributes() {
                    let attribute = attribute.map_err(|error| error.to_string())?;
                    if attribute.key.as_ref() == b"base" {
                        let value = attribute
                            .decode_and_unescape_value(reader.decoder())
                            .map_err(|error| error.to_string())?;
                        push_tokens(&mut references, ReferenceKind::ActorBase, &value)?;
                    }
                }
            }
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
                push_tokens(&mut references, kind, &value.unwrap_or_default())?;
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
    let extension = if reference.kind == ReferenceKind::ActorBase {
        ".actorparams"
    } else {
        ".xml"
    };
    if filename.strip_suffix(extension) != Some(name.as_str()) {
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
            | (ReferenceKind::ActorBase, ["libs", "actor", ..])
    )
}

/// A leveled actor names its per-level bases `BaseMonster1|BaseMonster2|…`.
/// Split such a token into the actor it may name and the level; the caller
/// must still confirm the actor declares that many `levels`.
pub fn leveled_base(reference: &DocumentReference) -> Option<(DocumentReference, u32)> {
    if reference.kind != ReferenceKind::ActorBase {
        return None;
    }
    let stem = reference
        .name
        .trim_end_matches(|c: char| c.is_ascii_digit());
    let level = reference.name[stem.len()..].parse().ok()?;
    (!stem.is_empty() && level > 0).then(|| {
        (
            DocumentReference {
                kind: ReferenceKind::ActorBase,
                name: stem.to_owned(),
            },
            level,
        )
    })
}

/// The root actor's authored `levels` count, if it declares one.
pub fn actor_levels(bytes: &[u8]) -> Result<Option<u32>, String> {
    let xml = crate::cryxml::xml_bytes(bytes, crate::gameplay_values::LIMIT)
        .map_err(|error| error.to_string())?;
    crate::cryxml::encode_xml(&xml, crate::gameplay_values::LIMIT)
        .map_err(|error| error.to_string())?;
    let mut reader = Reader::from_reader(xml.as_ref());
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(tag) | Event::Empty(tag) => {
                if tag.name().as_ref() != b"Actor" {
                    return Ok(None);
                }
                for attribute in tag.attributes() {
                    let attribute = attribute.map_err(|error| error.to_string())?;
                    if attribute.key.as_ref() == b"levels" {
                        let value = attribute
                            .decode_and_unescape_value(reader.decoder())
                            .map_err(|error| error.to_string())?;
                        return Ok(value.trim().parse().ok());
                    }
                }
                return Ok(None);
            }
            Event::Eof => return Ok(None),
            _ => (),
        }
    }
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
        // Legacy files a variant's documents in its base character's folder.
        let meteor_in_goliath = "Libs/Abilities/Goliath/GoliathMeteorCharge1.xml";
        assert!(owns_document("Meteor Goliath", meteor_in_goliath));
        assert!(!owns_document("Goliath", meteor_in_goliath));
        assert!(owns_document(
            "Goliath",
            "Libs/Abilities/Goliath/GoliathCharge1.xml"
        ));
        assert!(owns_document(
            "Blitz Markov",
            "Libs/Items/Mercs/Markov/MarkovBlitzRifle.xml"
        ));
        assert!(!owns_document(
            "Markov",
            "Libs/Items/Mercs/Markov/MarkovBlitzRifle.xml"
        ));
        // Only the base's own folder hands documents to a variant.
        assert!(!owns_document(
            "Meteor Goliath",
            "Libs/Abilities/Shared/GoliathMeteorCharge1.xml"
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
        assert_eq!(links.len(), 4);
        let base = links.iter().find(|link| link.name == "BaseMerc").unwrap();
        assert_eq!(base.kind, ReferenceKind::ActorBase);
        assert!(referenced_document(base, "libs/Actor/BaseMerc.actorparams"));
        assert!(!referenced_document(base, "libs/Items/BaseMerc.xml"));
        assert!(!referenced_document(
            base,
            "libs/Actor/BaseMercs.actorparams"
        ));
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
    fn actor_bases_come_only_from_the_root_actor_and_levels_need_a_declared_count() {
        let goliath = br#"<Actor name="GoliathA" levels="3" base="BaseMonster1|BaseMonster2|BaseMonster3"><Child base="NotAnActorBase"/></Actor>"#;
        let bases: Vec<_> = actor_references(goliath)
            .unwrap()
            .into_iter()
            .filter(|link| link.kind == ReferenceKind::ActorBase)
            .map(|link| link.name)
            .collect();
        assert_eq!(bases, ["BaseMonster1", "BaseMonster2", "BaseMonster3"]);
        let (actor, level) = leveled_base(&DocumentReference {
            kind: ReferenceKind::ActorBase,
            name: "BaseMonster3".into(),
        })
        .unwrap();
        assert_eq!((actor.name.as_str(), level), ("BaseMonster", 3));
        assert!(
            leveled_base(&DocumentReference {
                kind: ReferenceKind::ActorBase,
                name: "BaseMerc".into(),
            })
            .is_none()
        );
        assert!(
            leveled_base(&DocumentReference {
                kind: ReferenceKind::Item,
                name: "Gun2".into(),
            })
            .is_none()
        );
        assert_eq!(
            actor_levels(br#"<Actor name="BaseMonster" levels="3"/>"#).unwrap(),
            Some(3)
        );
        assert_eq!(actor_levels(br#"<Actor name="BaseMerc"/>"#).unwrap(), None);
        assert_eq!(actor_levels(br#"<Item levels="3"/>"#).unwrap(), None);
        assert!(
            actor_references(br#"<Item base="BaseMerc"/>"#)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn legacy_script_globals_belong_to_their_file_and_named_variant() {
        use RosterEntry::{Character, Wildlife};
        let goliath = "Scripts/Entities/actor/subtypes/subtypes_goliath.lua";
        assert_eq!(
            script_global_owner(goliath, "Goliath2"),
            Some(Character("Goliath"))
        );
        assert_eq!(
            script_global_owner(goliath, "GoliathA3"),
            Some(Character("Goliath"))
        );
        assert_eq!(
            script_global_owner(goliath, "GoliathMeteor1"),
            Some(Character("Meteor Goliath"))
        );
        for shared in ["GoliathBase", "GoliathMinion1", "Player"] {
            assert_eq!(script_global_owner(goliath, shared), None);
        }
        assert_eq!(
            script_global_owner(
                "Scripts/Entities/actor/subtypes/subtypes_grounder.lua",
                "KrakenElder2"
            ),
            Some(Character("Elder Kraken"))
        );
        let maggie = "Scripts/Entities/actor/subtype_maggie.lua";
        assert_eq!(
            script_global_owner(maggie, "TrackingE"),
            Some(Character("Maggie"))
        );
        assert_eq!(
            script_global_owner(maggie, "TrackingE_Savage"),
            Some(Character("Wasteland Maggie"))
        );
        // Variants missing from the roster are not given to the base.
        assert_eq!(
            script_global_owner(
                "Scripts/Entities/actor/subtype_hyde.lua",
                "AssaultD_Hellfire"
            ),
            None
        );
        // The same global in another hunter's file is that hunter's.
        assert_eq!(
            script_global_owner(
                "Scripts/Entities/actor/subtypes/subtype_tech.lua",
                "SupportF"
            ),
            Some(Character("Kala"))
        );
        assert_eq!(
            script_global_owner(
                "Scripts/Entities/AI/ArmadonBullElite_x.lua",
                "ArmadonBullElite_x"
            ),
            Some(Wildlife("Armadon Bull"))
        );
        assert_eq!(
            script_global_owner("Scripts/Entities/AI/TrapjawPet_x.lua", "TrapjawPet_x"),
            None
        );
        assert_eq!(
            script_global_owner("Scripts/Entities/actor/subtypes_merc.lua", "Tracking"),
            None
        );
        assert!(!roster_script("Scripts/Entities/AI/Trapjaw.lua"));
        assert!(!roster_script("Scripts/Entities/actor/player.lua"));
    }

    #[test]
    fn script_references_and_model_come_only_from_authored_fields() {
        use crate::lua51::{Table, Value};
        let table = |fields: Vec<(&str, Value)>, items: Vec<Value>| {
            Value::Table(Table {
                fields: fields
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect(),
                items,
            })
        };
        let text = |value: &str| Value::Str(value.into());
        let Value::Table(meteor) = table(
            vec![
                ("baseType", Value::Global("GoliathA1".into())),
                (
                    "loadout",
                    table(vec![], vec![text("GoliathMeteorCharge1"), text("../x")]),
                ),
                (
                    "gameParams",
                    table(
                        vec![
                            (
                                "effectSets",
                                table(vec![], vec![text("GoliathMeteorBurningSelf")]),
                            ),
                            (
                                "inventory",
                                table(
                                    vec![(
                                        "items",
                                        table(
                                            vec![(
                                                "claw",
                                                table(
                                                    vec![("name", text("GoliathMeteorClaw1"))],
                                                    vec![],
                                                ),
                                            )],
                                            vec![],
                                        ),
                                    )],
                                    vec![],
                                ),
                            ),
                        ],
                        vec![],
                    ),
                ),
                (
                    "Properties",
                    table(
                        vec![(
                            "fileModel",
                            text("Objects/GoliathMeteor/goliathmeteor1.cdf"),
                        )],
                        vec![],
                    ),
                ),
                ("description", text("GoliathClaw1")),
            ],
            vec![],
        ) else {
            unreachable!()
        };
        let references = script_references(&meteor);
        assert_eq!(references.len(), 3);
        assert!(
            references
                .iter()
                .any(|reference| reference.kind == ReferenceKind::Ability
                    && referenced_document(
                        reference,
                        "Libs/Abilities/Goliath/GoliathMeteorCharge1.xml"
                    ))
        );
        assert!(
            references
                .iter()
                .any(|reference| reference.kind == ReferenceKind::Item
                    && referenced_document(
                        reference,
                        "Libs/Items/Monsters/Goliath/GoliathMeteorClaw1.xml"
                    ))
        );
        assert!(
            references
                .iter()
                .any(|reference| reference.kind == ReferenceKind::EffectSet)
        );
        assert_eq!(
            script_model(&meteor).unwrap().as_deref(),
            Some("Objects/GoliathMeteor/goliathmeteor1.cdf")
        );
        let Value::Table(unsafe_model) = table(
            vec![(
                "Properties",
                table(vec![("fileModel", text("../outside.cdf"))], vec![]),
            )],
            vec![],
        ) else {
            unreachable!()
        };
        assert!(script_model(&unsafe_model).is_err());
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
    fn subtype_models_require_literal_safe_file_model_assignments() {
        let lua = b"-- fileModel = \"fake.cdf\"\nfileModel = \"Objects/Characters/Grounder/Grounder1.cdf\",\nclientFileModel = \"other.cdf\",\nfileModel = \"../outside.cdf\",\n";
        assert!(authored_subtype_models(lua).is_err());
        let valid = b"-- fileModel = \"fake.cdf\"\nfileModel = \"Objects/Characters/Grounder/Grounder1.cdf\",\nclientFileModel = \"other.cdf\",\n";
        assert_eq!(
            authored_subtype_models(valid).unwrap(),
            vec!["Objects/Characters/Grounder/Grounder1.cdf"]
        );
        assert!(authored_subtype_models(b"fileModel = \"safe.cdf\"\0").is_err());
    }

    #[test]
    fn cdf_materials_require_explicit_safe_bindings() {
        let xml = br#"<CharacterDefinition><Model File="objects/rig.chr" Material="objects/rig"/><AttachmentList><Attachment AName="body" Type="CA_SKIN" Binding="objects/body.skin" Material="objects/body.mtl"/></AttachmentList></CharacterDefinition>"#;
        let materials = authored_cdf_materials(xml).unwrap();
        assert_eq!(materials, vec!["objects/body.mtl", "objects/rig.mtl"]);
        assert!(authored_cdf_materials(br#"<CharacterDefinition><Model File="objects/rig.chr" Material="../other"/></CharacterDefinition>"#).is_err());
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

    #[test]
    #[ignore = "requires EVOLVE_LEGACY_LIBRARY pointing to an Asset Library with Legacy Scripts and GameData PAKs"]
    fn local_legacy_scripts_define_variants_wildlife_and_models() {
        let root = std::path::PathBuf::from(std::env::var_os("EVOLVE_LEGACY_LIBRARY").unwrap());
        let catalog = crate::library_catalog::imported_library(&root).unwrap();
        let mut owners = std::collections::BTreeMap::new();
        for archive in &catalog.archives {
            let inventory =
                crate::library_catalog::imported_archive_preview_inventory(archive).unwrap();
            for entry in inventory.entries {
                if !roster_script(&entry.relative_path) {
                    continue;
                }
                let bytes = crate::library_catalog::read_imported_member_bounded(
                    archive,
                    &entry.relative_path,
                    4 * 1024 * 1024,
                )
                .unwrap();
                for (global, table) in crate::lua51::authored_globals(&bytes).unwrap() {
                    if let Some(owner) = script_global_owner(&entry.relative_path, &global) {
                        let models = owners.entry(format!("{owner:?}")).or_insert_with(Vec::new);
                        models.extend(script_model(&table).unwrap());
                        if std::env::var_os("EVOLVE_LEGACY_VERBOSE").is_some() {
                            println!(
                                "{owner:?} {global} refs={}",
                                script_references(&table).len()
                            );
                        }
                    }
                }
            }
        }
        for expected in [
            "Character(\"Meteor Goliath\")",
            "Character(\"Wasteland Maggie\")",
            "Character(\"Electro Griffin\")",
            "Wildlife(\"Armadon Bull\")",
            "Wildlife(\"Trapjaw\")",
        ] {
            assert!(owners.contains_key(expected), "{expected} not defined");
        }
        assert!(owners["Character(\"Meteor Goliath\")"].iter().any(|model| {
            model.eq_ignore_ascii_case(
                "Objects/characters/Monsters/GoliathMeteor/goliathmeteor1.cdf",
            )
        }));
    }

    #[test]
    #[ignore = "requires EVOLVE_BENCH_LIBRARY pointing to a local imported Asset Library"]
    fn local_2012_subtype_assigns_a_real_grounder_cdf() {
        let root = std::path::PathBuf::from(std::env::var_os("EVOLVE_BENCH_LIBRARY").unwrap());
        let catalog = crate::library_catalog::imported_library(&root).unwrap();
        let archive = catalog
            .archives
            .iter()
            .find(|archive| archive.name == "Scripts.pak")
            .unwrap();
        let bytes = crate::library_catalog::read_imported_member_bounded(
            archive,
            "Scripts/Entities/actor/subtypes_grounder.lua",
            256 * 1024,
        )
        .unwrap();
        let models = authored_subtype_models(&bytes).unwrap();
        assert!(models.iter().any(|model| {
            model.eq_ignore_ascii_case("Objects/Characters/Monsters/Grounder/Grounder1.cdf")
        }));
    }

    #[test]
    #[ignore = "requires EVOLVE_BENCH_LIBRARY pointing to an Asset Library with Stage 2 scripts.pak"]
    fn local_stage2_subtypes_and_creatures_have_owned_models() {
        let root = std::path::PathBuf::from(std::env::var_os("EVOLVE_BENCH_LIBRARY").unwrap());
        let catalog = crate::library_catalog::imported_library(&root).unwrap();
        let mut owners = std::collections::BTreeMap::new();
        for archive in &catalog.archives {
            if !archive.name.eq_ignore_ascii_case("scripts.pak") {
                continue;
            }
            let inventory =
                crate::library_catalog::imported_archive_preview_inventory(archive).unwrap();
            for entry in inventory.entries {
                if !roster_script(&entry.relative_path) {
                    continue;
                }
                let bytes = crate::library_catalog::read_imported_member_bounded(
                    archive,
                    &entry.relative_path,
                    4 * 1024 * 1024,
                )
                .unwrap();
                if !crate::lua51::is_jee(&bytes) {
                    continue;
                }
                for (global, table) in crate::lua51::authored_globals(&bytes).unwrap() {
                    if let Some(owner) = script_global_owner(&entry.relative_path, &global) {
                        let models = owners.entry(format!("{owner:?}")).or_insert_with(Vec::new);
                        models.extend(script_model(&table).unwrap());
                    }
                }
            }
        }
        for expected in [
            "Character(\"Goliath\")",
            "Character(\"Meteor Goliath\")",
            "Character(\"Elder Kraken\")",
            "Wildlife(\"Armadon Bull\")",
        ] {
            assert!(
                owners.contains_key(expected),
                "{expected} not defined in Stage 2"
            );
        }
        assert!(owners["Character(\"Goliath\")"].iter().any(|model| {
            model.eq_ignore_ascii_case("characters/Monsters/Goliath/Goliath1.cdf")
        }));
        assert!(owners["Character(\"Meteor Goliath\")"].iter().any(|model| {
            model.eq_ignore_ascii_case("characters/Monsters/GoliathMeteor/goliathmeteor1.cdf")
        }));
        assert!(owners["Character(\"Elder Kraken\")"].iter().any(|model| {
            model.eq_ignore_ascii_case("characters/Monsters/KrakenElder/KrakenElder1.cdf")
        }));
        assert!(owners["Wildlife(\"Armadon Bull\")"].iter().any(|model| {
            model.eq_ignore_ascii_case("characters/creatures/armadonbull/ArmadonBull_Elite.cdf")
        }));
    }
}
