//! Evolve wildlife spawn placements as authored in level XML.
//!
//! The shipped maps use two systems (evidence in docs/WILDLIFE_SPAWNS.md):
//! fixed `WildlifeSpawnPoint`s gathered into `WildlifeGroup` circles that a
//! `WildlifeNode` chooses between through `<EntityLinks>`, and ZooKeeper
//! `ZooKeeperShape` areas whose keywords the DynamicSpawnRules match at
//! runtime. Older maps (Tutorial, murder_pits) place `CreatureSpawn`s.
//! This module only classifies and relates what the XML states; it never
//! simulates which candidate the game picks.
use crate::scene::{LevelScene, PlacedObject};
use std::collections::HashMap;

/// Bound on parsed `<Link>` targets per entity. Shipped nodes link at most 4.
pub const MAX_LINKS: usize = 64;

/// The four spawn behaviours a level designer works with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpawnType {
    /// One fixed point (`WildlifeSpawnPoint`, `CreatureSpawn`).
    Static,
    /// A node that links two or more candidate groups (usually three).
    Choice,
    /// A group circle (`fRadius`) holding its linked spawn points.
    Area,
    /// A painted ZooKeeper polygon filled by DynamicSpawnRules.
    Zone,
}

impl SpawnType {
    pub const ALL: [Self; 4] = [Self::Static, Self::Choice, Self::Area, Self::Zone];

    pub fn label(self) -> &'static str {
        match self {
            Self::Static => "Static points",
            Self::Choice => "Random sets",
            Self::Area => "Spawn circles",
            Self::Zone => "Dynamic zones",
        }
    }
}

/// An authored entity class that takes part in wildlife spawning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WildlifeEntity {
    SpawnPoint,
    CreatureSpawn,
    Group,
    Node,
    Zone,
}

impl WildlifeEntity {
    pub fn from_class(class: &str) -> Option<Self> {
        [
            Self::SpawnPoint,
            Self::CreatureSpawn,
            Self::Group,
            Self::Node,
            Self::Zone,
        ]
        .into_iter()
        .find(|entity| class.eq_ignore_ascii_case(entity.class_name()))
    }

    pub fn of(object: &PlacedObject) -> Option<Self> {
        Self::from_class(&object.kind)
    }

    pub fn class_name(self) -> &'static str {
        match self {
            Self::SpawnPoint => "WildlifeSpawnPoint",
            Self::CreatureSpawn => "CreatureSpawn",
            Self::Group => "WildlifeGroup",
            Self::Node => "WildlifeNode",
            Self::Zone => "ZooKeeperShape",
        }
    }

    pub fn spawn_type(self) -> SpawnType {
        match self {
            Self::SpawnPoint | Self::CreatureSpawn => SpawnType::Static,
            Self::Node => SpawnType::Choice,
            Self::Group => SpawnType::Area,
            Self::Zone => SpawnType::Zone,
        }
    }

    /// Mission entities whose script properties live on a single
    /// `<Properties><extensionfolderbegin<Class> .../></Properties>` child.
    pub fn has_extension_properties(self) -> bool {
        matches!(self, Self::SpawnPoint | Self::Group | Self::Node)
    }

    /// The class this entity's `<EntityLinks>` point at, in shipped data.
    pub fn link_target(self) -> Option<Self> {
        match self {
            Self::Node => Some(Self::Group),
            Self::Group => Some(Self::SpawnPoint),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkRole {
    /// WildlifeNode → one of its candidate WildlifeGroups.
    Candidate,
    /// WildlifeGroup → a WildlifeSpawnPoint it holds.
    Member,
    /// CreatureSpawn → the ZooKeeperShape named by its HomeAreaName.
    Home,
}

/// A relation between two scene objects, by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WildlifeLink {
    pub from: usize,
    pub to: usize,
    pub role: LinkRole,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WildlifeLayout {
    pub links: Vec<WildlifeLink>,
    /// Group circles: (object index, authored fRadius).
    pub circles: Vec<(usize, f32)>,
    /// Links whose target is missing from the loaded scene or has the
    /// wrong class; shown as a diagnostic, never guessed.
    pub unresolved_links: usize,
}

/// Relate the visible wildlife entities of a scene. Hidden objects are left
/// out so the overlay matches what the viewport draws.
pub fn layout(scene: &LevelScene) -> WildlifeLayout {
    let by_id = scene
        .objects
        .iter()
        .enumerate()
        .filter(|(_, object)| !object.hidden)
        .map(|(index, object)| (object.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let zones = scene
        .objects
        .iter()
        .enumerate()
        .filter(|(_, object)| {
            !object.hidden && WildlifeEntity::of(object) == Some(WildlifeEntity::Zone)
        })
        .map(|(index, object)| (object.name.to_ascii_lowercase(), index))
        .collect::<HashMap<_, _>>();
    let mut out = WildlifeLayout::default();
    for (index, object) in scene.objects.iter().enumerate() {
        if object.hidden {
            continue;
        }
        let Some(entity) = WildlifeEntity::of(object) else {
            continue;
        };
        if entity == WildlifeEntity::Group
            && let Some(radius) = object.wildlife.radius.filter(|r| *r > 0.0)
        {
            out.circles.push((index, radius));
        }
        if entity == WildlifeEntity::CreatureSpawn && !object.home_area.is_empty() {
            match zones.get(&object.home_area.to_ascii_lowercase()) {
                Some(&home) => out.links.push(WildlifeLink {
                    from: index,
                    to: home,
                    role: LinkRole::Home,
                }),
                None => out.unresolved_links += 1,
            }
        }
        let Some(expected) = entity.link_target() else {
            continue;
        };
        let role = if entity == WildlifeEntity::Node {
            LinkRole::Candidate
        } else {
            LinkRole::Member
        };
        for target in &object.wildlife.links {
            match by_id.get(target.as_str()) {
                Some(&to) if WildlifeEntity::of(&scene.objects[to]) == Some(expected) => {
                    out.links.push(WildlifeLink {
                        from: index,
                        to,
                        role,
                    });
                }
                _ => out.unresolved_links += 1,
            }
        }
    }
    out
}

/// Indices of objects whose `<EntityLinks>` name `id`.
pub fn linked_from(scene: &LevelScene, id: &str) -> Vec<usize> {
    scene
        .objects
        .iter()
        .enumerate()
        .filter(|(_, object)| {
            object
                .wildlife
                .links
                .iter()
                .any(|target| target.eq_ignore_ascii_case(id))
        })
        .map(|(index, _)| index)
        .collect()
}

/// Editable settings of one wildlife entity, by class.
#[derive(Clone, Debug, PartialEq)]
pub enum WildlifeSettings {
    SpawnPoint {
        species: String,
    },
    Group {
        radius: f32,
    },
    Node {
        enabled: bool,
        category: String,
        /// Only some shipped nodes author `fRadius`; None leaves it absent.
        radius: Option<f32>,
    },
    Zone {
        keywords: String,
        use_terrain_height: bool,
        restrict_all_species: bool,
    },
}

/// Accepted range for a group or node `fRadius`. Shipped values are 8–20 m.
pub const RADIUS_RANGE: std::ops::RangeInclusive<f32> = 0.5..=500.0;

fn identifier(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Normalise a keyword list to the shipped `A, B` spelling.
pub fn normalize_keywords(raw: &str) -> Result<String, String> {
    let words = raw
        .split(',')
        .map(str::trim)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    if words.len() > 16 || words.iter().any(|word| !identifier(word)) {
        return Err(
            "Zone keywords are up to 16 comma-separated names of letters, digits and _".into(),
        );
    }
    Ok(words.join(", "))
}

impl WildlifeSettings {
    pub fn entity(&self) -> WildlifeEntity {
        match self {
            Self::SpawnPoint { .. } => WildlifeEntity::SpawnPoint,
            Self::Group { .. } => WildlifeEntity::Group,
            Self::Node { .. } => WildlifeEntity::Node,
            Self::Zone { .. } => WildlifeEntity::Zone,
        }
    }

    /// The settings an object holds now, if its class has any here.
    pub fn of(object: &PlacedObject) -> Option<Self> {
        let w = &object.wildlife;
        Some(match WildlifeEntity::of(object)? {
            WildlifeEntity::SpawnPoint => Self::SpawnPoint {
                species: w.species.clone(),
            },
            WildlifeEntity::Group => Self::Group {
                radius: w.radius.unwrap_or(10.0),
            },
            WildlifeEntity::Node => Self::Node {
                enabled: w.enabled.unwrap_or(true),
                category: if w.category.is_empty() {
                    "Default".into()
                } else {
                    w.category.clone()
                },
                radius: w.radius,
            },
            WildlifeEntity::Zone => Self::Zone {
                keywords: w.keywords.clone(),
                use_terrain_height: w.use_terrain_height.unwrap_or(false),
                restrict_all_species: w.restrict_species_to_area.unwrap_or(false),
            },
            WildlifeEntity::CreatureSpawn => return None,
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        let radius = |r: f32| {
            if r.is_finite() && RADIUS_RANGE.contains(&r) {
                Ok(())
            } else {
                Err(format!(
                    "Radius must be between {} and {} m",
                    RADIUS_RANGE.start(),
                    RADIUS_RANGE.end()
                ))
            }
        };
        match self {
            Self::SpawnPoint { species } if !identifier(species) => {
                Err("Species must be one esWildlife name such as Reaver".into())
            }
            Self::Group { radius: r } => radius(*r),
            Self::Node {
                category,
                radius: r,
                ..
            } => {
                if !identifier(category) {
                    return Err("Category must be a name such as Default or LargeWildlife".into());
                }
                r.map_or(Ok(()), radius)
            }
            Self::Zone { keywords, .. } => {
                if normalize_keywords(keywords)? != *keywords || keywords.is_empty() {
                    return Err("Enter at least one zone keyword, separated by commas".into());
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Whether a re-parsed object carries exactly these settings.
    pub fn matches(&self, object: &PlacedObject) -> bool {
        Self::of(object).as_ref() == Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::parse_level_scene;
    use crate::scene_authoring::{LayerAuthoringEdit, edit_layer_bytes};

    const MISSION: &[u8] = br#"<Mission><Objects>
<Entity Name="Node_A" Pos="10,0,0" EntityClass="WildlifeNode" EntityId="1" EntityGuid="4000000000000001" Layer="WildlifeSpawns"><EntityLinks><Link TargetId="2" Name="NewLink" RelRot="1,0,0,0" RelPos="0,0,0"/><Link TargetId="3" Name="NewLink" RelRot="1,0,0,0" RelPos="0,0,0"/><Link TargetId="99" Name="NewLink" RelRot="1,0,0,0" RelPos="0,0,0"/></EntityLinks><Properties><extensionfolderbeginWildlifeNode bEnabled="1" esWildlifeCategory="LargeWildlife"/></Properties></Entity>
<Entity Name="Group_B" Pos="0,0,0" EntityClass="WildlifeGroup" EntityId="2" EntityGuid="4000000000000002" Layer="WildlifeSpawns"><EntityLinks><Link TargetId="4" Name="NewLink" RelRot="1,0,0,0" RelPos="0,0,0"/></EntityLinks><Properties><extensionfolderbeginWildlifeGroup fRadius="10"/></Properties></Entity>
<Entity Name="Group_C" Pos="20,0,0" EntityClass="WildlifeGroup" EntityId="3" EntityGuid="4000000000000003" Layer="WildlifeSpawns"><EntityLinks><Link TargetId="5" Name="NewLink" RelRot="1,0,0,0" RelPos="0,0,0"/></EntityLinks><Properties><extensionfolderbeginWildlifeGroup fRadius="8"/></Properties></Entity>
<Entity Name="Point_D" Pos="1,1,0" EntityClass="WildlifeSpawnPoint" EntityId="4" EntityGuid="4000000000000004" Layer="WildlifeSpawns"><Properties><extensionfolderbeginWildlifeSpawnPoint esWildlife="Reaver"/></Properties></Entity>
<Entity Name="Point_E" Pos="21,1,0" EntityClass="WildlifeSpawnPoint" EntityId="5" EntityGuid="4000000000000005" Layer="WildlifeSpawns"><Properties><extensionfolderbeginWildlifeSpawnPoint esWildlife="MarshStrider"/></Properties></Entity>
<Entity Name="Zone_F" Pos="0,50,0" EntityClass="ZooKeeperShape" EntityId="6" EntityGuid="4000000000000006" Layer="ZooKeepers"><Area ZooKeeperKeywords="ArmadonBull, NoElites" ZooKeeperUseTerrainHeight="1" ZooKeeperRestrictAllSpeciesToArea="0"><Points><Point Pos="0,0,0"/><Point Pos="5,0,0"/><Point Pos="5,5,0"/></Points></Area></Entity>
</Objects></Mission>"#;

    #[test]
    fn mission_wildlife_classes_settings_and_links_parse() {
        let scene = parse_level_scene(&[("mission", MISSION)]).unwrap();
        let kinds = scene
            .objects
            .iter()
            .map(|o| WildlifeEntity::of(o).map(WildlifeEntity::spawn_type))
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [
                Some(SpawnType::Choice),
                Some(SpawnType::Area),
                Some(SpawnType::Area),
                Some(SpawnType::Static),
                Some(SpawnType::Static),
                Some(SpawnType::Zone),
            ]
        );
        assert!(scene.objects.iter().all(PlacedObject::is_wildlife_spawn));
        assert_eq!(scene.objects[0].wildlife.links, ["2", "3", "99"]);
        assert_eq!(
            WildlifeSettings::of(&scene.objects[0]),
            Some(WildlifeSettings::Node {
                enabled: true,
                category: "LargeWildlife".into(),
                radius: None
            })
        );
        assert_eq!(scene.objects[2].wildlife.radius, Some(8.0));
        assert_eq!(scene.objects[3].wildlife.species, "Reaver");
        assert_eq!(scene.objects[5].wildlife.keywords, "ArmadonBull, NoElites");
    }

    #[test]
    fn layout_relates_nodes_groups_points_and_counts_dangling_links() {
        let scene = parse_level_scene(&[("mission", MISSION)]).unwrap();
        let layout = layout(&scene);
        assert_eq!(layout.circles, [(1, 10.0), (2, 8.0)]);
        assert_eq!(layout.unresolved_links, 1);
        assert!(layout.links.contains(&WildlifeLink {
            from: 0,
            to: 2,
            role: LinkRole::Candidate
        }));
        assert!(layout.links.contains(&WildlifeLink {
            from: 1,
            to: 3,
            role: LinkRole::Member
        }));
        assert_eq!(linked_from(&scene, "4"), [1]);
        assert!(linked_from(&scene, "6").is_empty());
    }

    fn edit(edit: &LayerAuthoringEdit) -> Result<LevelScene, crate::scene::SceneError> {
        let bytes = edit_layer_bytes(MISSION, edit)?;
        parse_level_scene(&[("edited", &bytes)])
    }

    #[test]
    fn settings_round_trip_through_the_layer_writer() {
        let scene = edit(&LayerAuthoringEdit::Wildlife {
            guid: "2".into(),
            settings: WildlifeSettings::Group { radius: 14.5 },
        })
        .unwrap();
        assert_eq!(scene.objects[1].wildlife.radius, Some(14.5));
        assert_eq!(scene.objects[1].wildlife.links, ["4"]);
        let node = WildlifeSettings::Node {
            enabled: false,
            category: "EliteTier2".into(),
            radius: Some(20.0),
        };
        let scene = edit(&LayerAuthoringEdit::Wildlife {
            guid: "1".into(),
            settings: node.clone(),
        })
        .unwrap();
        assert!(node.matches(&scene.objects[0]));
        let zone = WildlifeSettings::Zone {
            keywords: "ForestAquatic".into(),
            use_terrain_height: false,
            restrict_all_species: true,
        };
        let scene = edit(&LayerAuthoringEdit::Wildlife {
            guid: "6".into(),
            settings: zone.clone(),
        })
        .unwrap();
        assert!(zone.matches(&scene.objects[5]));
        assert_eq!(scene.objects[5].shape_points.len(), 3);
        let scene = edit(&LayerAuthoringEdit::Wildlife {
            guid: "4".into(),
            settings: WildlifeSettings::SpawnPoint {
                species: "Cephaladon".into(),
            },
        })
        .unwrap();
        assert_eq!(scene.objects[3].wildlife.species, "Cephaladon");
    }

    #[test]
    fn settings_refuse_the_wrong_class_and_invalid_values() {
        assert!(
            edit(&LayerAuthoringEdit::Wildlife {
                guid: "4".into(),
                settings: WildlifeSettings::Group { radius: 5.0 },
            })
            .is_err()
        );
        assert!(
            edit(&LayerAuthoringEdit::Wildlife {
                guid: "2".into(),
                settings: WildlifeSettings::Group { radius: -1.0 },
            })
            .is_err()
        );
    }

    #[test]
    fn links_keep_retained_targets_add_new_ones_and_refuse_wrong_classes() {
        let scene = edit(&LayerAuthoringEdit::Links {
            guid: "1".into(),
            targets: vec!["3".into(), "2".into()],
        })
        .unwrap();
        assert_eq!(scene.objects[0].wildlife.links, ["3", "2"]);
        let scene = edit(&LayerAuthoringEdit::Links {
            guid: "2".into(),
            targets: vec!["4".into(), "5".into()],
        })
        .unwrap();
        assert_eq!(scene.objects[1].wildlife.links, ["4", "5"]);
        for targets in [
            vec![],
            vec!["4".to_owned()],
            vec!["2".to_owned(), "2".to_owned()],
            vec!["77".to_owned()],
        ] {
            assert!(
                edit(&LayerAuthoringEdit::Links {
                    guid: "1".into(),
                    targets,
                })
                .is_err()
            );
        }
        // A spawn point has no links of its own.
        assert!(
            edit(&LayerAuthoringEdit::Links {
                guid: "4".into(),
                targets: vec!["5".into()],
            })
            .is_err()
        );
    }

    #[test]
    fn linked_targets_cannot_be_deleted_but_unlinked_ones_can() {
        assert!(edit(&LayerAuthoringEdit::Delete { guid: "2".into() }).is_err());
        assert!(edit(&LayerAuthoringEdit::Delete { guid: "4".into() }).is_err());
        let scene = edit(&LayerAuthoringEdit::Delete { guid: "1".into() }).unwrap();
        assert_eq!(scene.objects.len(), 5);
    }

    /// Summarise a real extracted mission or ExternalLayers XML:
    /// `EVOLVE_WILDLIFE_XML=<file> cargo test -p evolve-core shipped_wildlife -- --ignored --nocapture`
    #[test]
    #[ignore = "requires EVOLVE_WILDLIFE_XML pointing to an extracted mission_*.xml"]
    fn shipped_wildlife_layout_summary() {
        let path = std::env::var("EVOLVE_WILDLIFE_XML").unwrap();
        let bytes = std::fs::read(path).unwrap();
        let scene = parse_level_scene(&[("shipped", &bytes)]).unwrap();
        let mut counts = HashMap::<SpawnType, usize>::new();
        for object in &scene.objects {
            if let Some(entity) = WildlifeEntity::of(object) {
                *counts.entry(entity.spawn_type()).or_default() += 1;
            }
        }
        let layout = layout(&scene);
        println!(
            "{counts:?}; {} links, {} circles, {} unresolved",
            layout.links.len(),
            layout.circles.len(),
            layout.unresolved_links
        );
        // Shipped data is not perfectly linked: Tutorial names home areas
        // that its mission lacks, and Forest_Cliffs has one group→group link.
        assert!(!layout.links.is_empty() || counts.contains_key(&SpawnType::Zone));
        if let Some(&(index, _)) = layout.circles.first() {
            let group = &scene.objects[index];
            let edited = edit_layer_bytes(
                &bytes,
                &LayerAuthoringEdit::Wildlife {
                    guid: group.id.clone(),
                    settings: WildlifeSettings::Group { radius: 12.0 },
                },
            )
            .unwrap();
            let reparsed = parse_level_scene(&[("edited", &edited)]).unwrap();
            assert_eq!(reparsed.objects[index].wildlife.radius, Some(12.0));
            assert_eq!(reparsed.objects.len(), scene.objects.len());
        }
    }

    #[test]
    fn settings_validation_refuses_out_of_range_and_unsafe_names() {
        assert!(WildlifeSettings::Group { radius: 0.0 }.validate().is_err());
        assert!(
            WildlifeSettings::Group { radius: f32::NAN }
                .validate()
                .is_err()
        );
        assert!(WildlifeSettings::Group { radius: 12.0 }.validate().is_ok());
        assert!(
            WildlifeSettings::SpawnPoint {
                species: "Re aver\"".into()
            }
            .validate()
            .is_err()
        );
        assert_eq!(
            normalize_keywords(" ArmadonBull,NoElites ,").unwrap(),
            "ArmadonBull, NoElites"
        );
        assert!(normalize_keywords("Bad Word").is_err());
        assert!(
            WildlifeSettings::Zone {
                keywords: String::new(),
                use_terrain_height: true,
                restrict_all_species: false
            }
            .validate()
            .is_err()
        );
    }
}
