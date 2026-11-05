//! Read-only, data-backed ZooKeeper population and spawn-distance lookup.
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::collections::{HashMap, HashSet};

const MAX_XML_BYTES: usize = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;
const MAX_BASE_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PopulationKind {
    Creature,
    Npc,
    Other,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPopulation {
    pub name: String,
    pub archetype: Option<String>,
    pub class: Option<String>,
    pub model: Option<String>,
    pub kind: PopulationKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnDistances {
    pub minimum: f32,
    pub maximum: f32,
    pub despawn: f32,
    pub enable_ai: Option<f32>,
    pub disable_ai: Option<f32>,
}

#[derive(Clone, Debug, Default)]
struct Population {
    name: String,
    base: Option<String>,
    archetype: Option<String>,
    class: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ZooKeeper {
    populations: HashMap<String, Population>,
    archetypes: HashMap<String, String>,
    pub distances: Option<SpawnDistances>,
}

fn attributes(node: &BytesStart<'_>) -> Result<HashMap<String, String>, String> {
    node.attributes()
        .map(|attr| {
            let attr = attr.map_err(|e| e.to_string())?;
            let value = attr.unescape_value().map_err(|e| e.to_string())?;
            Ok((
                String::from_utf8_lossy(attr.key.as_ref()).to_ascii_lowercase(),
                value.into_owned(),
            ))
        })
        .collect()
}

fn field(attrs: &HashMap<String, String>, name: &str) -> Option<String> {
    attrs.get(name).filter(|s| !s.trim().is_empty()).cloned()
}

fn walk(
    xml: &[u8],
    mut visit: impl FnMut(
        &[u8],
        &HashMap<String, String>,
        &[(String, HashMap<String, String>)],
    ) -> Result<(), String>,
) -> Result<(), String> {
    if xml.len() > MAX_XML_BYTES {
        return Err("ZooKeeper XML exceeds size limit".into());
    }
    let mut reader = Reader::from_reader(xml);
    let mut parents = Vec::<(String, HashMap<String, String>)>::new();
    let mut count = 0;
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(node) => {
                count += 1;
                if count > MAX_ENTRIES || parents.len() > 64 {
                    return Err("ZooKeeper XML exceeds entry/depth limit".into());
                }
                let attrs = attributes(&node)?;
                visit(node.name().as_ref(), &attrs, &parents)?;
                parents.push((
                    String::from_utf8_lossy(node.name().as_ref()).into_owned(),
                    attrs,
                ));
            }
            Event::Empty(node) => {
                count += 1;
                if count > MAX_ENTRIES || parents.len() > 64 {
                    return Err("ZooKeeper XML exceeds entry/depth limit".into());
                }
                visit(node.name().as_ref(), &attributes(&node)?, &parents)?;
            }
            Event::End(_) => {
                parents.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(())
}

impl ZooKeeper {
    pub fn from_xml(
        populations: &[u8],
        archetypes: &[u8],
        spawning: &[u8],
    ) -> Result<Self, String> {
        let mut result = Self::default();
        walk(populations, |tag, attrs, _| {
            if tag.eq_ignore_ascii_case(b"Population")
                && let Some(name) = field(attrs, "name")
            {
                result.populations.insert(
                    name.to_ascii_lowercase(),
                    Population {
                        name,
                        base: field(attrs, "base"),
                        archetype: field(attrs, "archetype"),
                        class: field(attrs, "class"),
                    },
                );
            }
            Ok(())
        })?;
        walk(archetypes, |tag, attrs, parents| {
            if tag.eq_ignore_ascii_case(b"Properties")
                && let Some((_, prototype)) = parents
                    .last()
                    .filter(|(tag, _)| tag.eq_ignore_ascii_case("EntityPrototype"))
                && let Some(name) = field(prototype, "name")
                && let Some(model) =
                    field(attrs, "filemodel").or_else(|| field(attrs, "clientfilemodel"))
            {
                result
                    .archetypes
                    .insert(format!("ai.{name}").to_ascii_lowercase(), model);
            }
            Ok(())
        })?;
        let mut spawn = HashMap::new();
        walk(spawning, |tag, attrs, parents| {
            if tag.eq_ignore_ascii_case(b"SpawnPoint") {
                if let Some(distances) = SpawnDistances::from_params(&spawn) {
                    result.distances = Some(distances);
                }
                spawn = attrs.clone();
            } else if tag.eq_ignore_ascii_case(b"param")
                && parents
                    .last()
                    .is_some_and(|(name, _)| name.eq_ignore_ascii_case("SpawnPoint"))
                && let (Some(name), Some(value)) = (field(attrs, "name"), field(attrs, "value"))
            {
                spawn.insert(name.to_ascii_lowercase(), value);
            }
            Ok(())
        })?;
        if let Some(distances) = SpawnDistances::from_params(&spawn) {
            result.distances = Some(distances);
        }
        Ok(result)
    }

    pub fn resolve(&self, name: &str) -> Result<ResolvedPopulation, String> {
        let mut visited = HashSet::new();
        let mut current = name.to_ascii_lowercase();
        let mut archetype = None;
        let mut class = None;
        let mut canonical = None;
        for _ in 0..MAX_BASE_DEPTH {
            if !visited.insert(current.clone()) {
                return Err(format!("Cyclic population base: {name}"));
            }
            let pop = self
                .populations
                .get(&current)
                .ok_or_else(|| format!("Unknown population: {name}"))?;
            if canonical.is_none() {
                canonical = Some(pop.name.clone());
            }
            if archetype.is_none() {
                archetype = pop.archetype.clone();
            }
            if class.is_none() {
                class = pop.class.clone();
            }
            if pop.base.is_none() {
                break;
            }
            current = pop.base.as_deref().unwrap_or_default().to_ascii_lowercase();
        }
        if visited.len() == MAX_BASE_DEPTH {
            return Err(format!("Population base depth exceeded: {name}"));
        }
        let kind = match archetype.as_deref().map(str::to_ascii_lowercase) {
            Some(s) if s.starts_with("ai.creatures.") => PopulationKind::Creature,
            Some(s) if s.starts_with("ai.npc") => PopulationKind::Npc,
            Some(_) => PopulationKind::Other,
            None => PopulationKind::Unknown,
        };
        let model = archetype
            .as_ref()
            .and_then(|a| self.archetypes.get(&a.to_ascii_lowercase()))
            .cloned();
        Ok(ResolvedPopulation {
            name: canonical.unwrap_or_default(),
            archetype,
            class,
            model,
            kind,
        })
    }
}

impl SpawnDistances {
    fn from_params(params: &HashMap<String, String>) -> Option<Self> {
        if !params.get("sclasses")?.eq_ignore_ascii_case("Default") {
            return None;
        }
        let number = |key: &str| {
            params
                .get(key)
                .and_then(|s| s.parse::<f32>().ok())
                .filter(|v| v.is_finite() && *v >= 0.0)
        };
        Some(Self {
            minimum: number("fspawndistancemin")?,
            maximum: number("fspawndistancemax")?,
            despawn: number("fdespawndistance")?,
            enable_ai: number("fenableaidistance"),
            disable_ai: number("fdisableaidistance"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POPULATIONS: &[u8] = br#"<ZooKeeper><Populations>
        <Population name="Trapjaw" class="Trapjaw"/>
        <Population name="Trapjaw_Tutorial" base="Trapjaw" archetype="AI.Creatures.Trapjaw_Tutorial"/>
        <Population name="TyrantElite_Tutorial" base="TyrantElite" archetype="AI.Creatures.TyrantElite_Tutorial"/>
        <Population name="TyrantElite" class="TyrantElite"/>
        <Population name="NPC" archetype="AI.NPC_Combatant.EbonStar_M01"/>
        <Population name="CycleA" base="CycleB"/>
        <Population name="CycleB" base="CycleA"/>
        <Population name="Broken" base="Missing"/>
    </Populations></ZooKeeper>"#;
    const ARCHETYPES: &[u8] = br#"<EntityPrototypeLibrary Name="AI">
        <EntityPrototype Name="Creatures.Trapjaw_Tutorial"><Properties fileModel="characters/creatures/trapjaw/trapjaw_wild.cdf"/></EntityPrototype>
        <EntityPrototype Name="Creatures.TyrantElite_Tutorial"><Properties clientFileModel="characters/creatures/tyrant/tyrant_elite.cdf"/></EntityPrototype>
        <EntityPrototype Name="NPC_Combatant.EbonStar_M01"><Properties fileModel="characters/hunters/npc.cdf"/></EntityPrototype>
    </EntityPrototypeLibrary>"#;
    const SPAWNING: &[u8] = br#"<WildlifeSpawning><SpawnPoints><SpawnPoint>
        <param name="sClasses" value="Default"/><param name="fSpawnDistanceMin" value="30"/>
        <param name="fSpawnDistanceMax" value="110"/><param name="fDespawnDistance" value="120"/>
        <param name="fEnableAIDistance" value="50"/><param name="fDisableAIDistance" value="70"/>
        </SpawnPoint><SpawnPoint><param name="sClasses" value="Other"/><param name="fSpawnDistanceMin" value="900"/></SpawnPoint>
    </SpawnPoints></WildlifeSpawning>"#;

    #[test]
    fn resolves_data_not_guessed_filenames() {
        let data = ZooKeeper::from_xml(POPULATIONS, ARCHETYPES, SPAWNING).unwrap();
        let trapjaw = data.resolve("tRAPJAW_tUTORIAL").unwrap();
        assert_eq!(trapjaw.class.as_deref(), Some("Trapjaw"));
        assert_eq!(trapjaw.kind, PopulationKind::Creature);
        assert_eq!(
            trapjaw.model.as_deref(),
            Some("characters/creatures/trapjaw/trapjaw_wild.cdf")
        );
        assert_eq!(
            data.resolve("tyrantelite_tutorial")
                .unwrap()
                .model
                .as_deref(),
            Some("characters/creatures/tyrant/tyrant_elite.cdf")
        );
        assert_eq!(data.resolve("npc").unwrap().kind, PopulationKind::Npc);
        assert_eq!(data.resolve("trapjaw").unwrap().model, None);
        assert!(data.resolve("Missing").is_err());
        assert!(data.resolve("CycleA").is_err());
        assert!(data.resolve("Broken").is_err());
        assert_eq!(
            data.distances.unwrap(),
            SpawnDistances {
                minimum: 30.0,
                maximum: 110.0,
                despawn: 120.0,
                enable_ai: Some(50.0),
                disable_ai: Some(70.0)
            }
        );
    }
}
