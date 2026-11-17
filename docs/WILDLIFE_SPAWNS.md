# Wildlife spawn data in Evolve levels

How the four kinds of wildlife spawn a level designer works with are stored in
the shipped level XML, and what the Level Editor reads, draws and writes. The
evidence comes from the extracted Stage 2 `level.pak` mission files
(`levels/<Map>/mission_mission0.xml`), the `levels_<map>.pak` external layers
and the `GameData` ZooKeeper libraries. Mission XML is plain text, so
"offsets" below are element and attribute paths, not byte offsets.

Parsing lives in `evolve-core` (`scene.rs`, `wildlife_spawns.rs`); writing in
`scene_authoring.rs` (`LayerAuthoringEdit::Wildlife` and `::Links`). The
desktop crate only draws and edits through those.

## The four spawn types

| Type (editor name) | Entity class | Key data |
| --- | --- | --- |
| Static point (*Points*) | `WildlifeSpawnPoint`; older maps `CreatureSpawn` | `Pos`; `esWildlife` species |
| Random set (*Sets*) | `WildlifeNode` | `<EntityLinks>` to 2–4 `WildlifeGroup`s (usually 3) |
| Spawn circle (*Circles*) | `WildlifeGroup` | `fRadius`; `<EntityLinks>` to its `WildlifeSpawnPoint`s |
| Dynamic zone (*Zones*) | `ZooKeeperShape` | polygon `Points`; `ZooKeeperKeywords` |

### WildlifeSpawnPoint, WildlifeGroup, WildlifeNode

All three are mission entities on `Layer="WildlifeSpawns"`
(`<Mission><Objects><Entity EntityClass=… EntityId=… EntityGuid=…>`). Their
script properties are one element under `Properties`:

```xml
<Entity Name="WildlifeSpawnPoint_Reaver_3" EntityClass="WildlifeSpawnPoint" EntityId="…" Layer="WildlifeSpawns" Pos="…">
  <Properties><extensionfolderbeginWildlifeSpawnPoint esWildlife="Reaver"/></Properties>
</Entity>
<Entity Name="WildlifeGroup_Reaver_3" EntityClass="WildlifeGroup" ParentId="…" Pos="…">
  <EntityLinks><Link TargetId="<spawn point EntityId>" Name="NewLink" RelRot="1,0,0,0" RelPos="0,0,0"/></EntityLinks>
  <Properties><extensionfolderbeginWildlifeGroup fRadius="10"/></Properties>
</Entity>
<Entity Name="WildlifeNode_12" EntityClass="WildlifeNode" ParentId="…" Pos="…">
  <EntityLinks><Link TargetId="<group EntityId>" …/> ×3</EntityLinks>
  <Properties><extensionfolderbeginWildlifeNode bEnabled="1" esWildlifeCategory="LargeWildlife"/></Properties>
</Entity>
```

- `Properties/extensionfolderbeginWildlifeSpawnPoint@esWildlife`: species
  name (Reaver, MarshStrider, ArmadonBull, …).
- `Properties/extensionfolderbeginWildlifeGroup@fRadius`: 8 or 10 in the
  shipped missions. Linked points sit 0.2–2× that distance from the group.
- `Properties/extensionfolderbeginWildlifeNode@bEnabled`,
  `@esWildlifeCategory` (Default, LargeWildlife, EliteTier1–3). Some
  external-layer nodes also carry `fRadius="20"`.
- `EntityLinks/Link@TargetId` is the target's `EntityId`. All 1572 shipped
  links checked use `Name="NewLink" RelRot="1,0,0,0" RelPos="0,0,0"`, which
  is the form the editor writes for a new link.
- Nodes link groups, groups link spawn points. One link in the Forest_Cliffs
  external layer goes group → group; the editor counts it as unresolved
  and does not write that form.
- `ParentId` often attaches a group to a spawn point and a node to a group.
  This attachment is separate from the links.

Counts in the Stage 2 mission files:

| Map | Groups | Nodes | Points | ZooKeeperShapes | Links resolved |
| --- | --- | --- | --- | --- | --- |
| Desert_Canyon | 36 | 32 | 55 | – | 91, 0 unresolved |
| Ice_Crashsite | 119 | 54 | 151 | 73 | 269, 0 unresolved |
| Lava_Rivers | 61 | 35 | 93 | – | – |

**External layers.** Several maps keep their spawn entities outside the
mission file, in
`levels_<map>.pak: levels/<Map>/ExternalLayers/{main|mission}/WildlifeSpawns.xml`
(root `<Mission><Objects>`, same entity form). This applies to Desert_Mountains,
Forest_Cliffs, Forest_Rapids, Forest_Ruins, Ice_Alpine, Ice_Crashsite and
Lava_Rivers. Variants such as `WildlifeSpawns_Large.xml` and
`DD_WildlifeSpawns.xml` also exist. They share `EntityGuid`s with the
mission entities but use different `EntityId`s. Forest_Cliffs and Ice_Alpine
have no spawn points in the mission file at all. The Forest_Cliffs external
layer parses to 101 groups, 47 nodes, 146 points and 244 links, with 2
unresolved.

### ZooKeeperShape (dynamic zones)

```xml
<Entity Name="…" EntityClass="ZooKeeperShape" Layer="ZooKeepers" Pos="…">
  <Area ZooKeeperKeywords="ArmadonBull, NoElites" ZooKeeperUseTerrainHeight="1"
        ZooKeeperRestrictAllSpeciesToArea="0" …>
    <Points><Point Pos="x,y,z"/>…</Points>
  </Area>
</Entity>
```

The polygon is local to the entity transform. `GameData:
Libs/ZooKeeper/DynamicSpawnRules*.xml` declares `Zones`, `Rules` and
`PopulationConstraints`. Rule `AreaKeywords Inside="MonsterSpawn"` entries
match these shape keywords, and the file sets `grid_size` 100. Ice_Crashsite
has 73 shapes and the Forest_Cliffs external layers have 79.

### CreatureSpawn (older static points)

Tutorial (88) and murder_pits (51) use `CreatureSpawn`, with
`Properties/Populations` (Population1–4) and `HomeAreaName` naming a
ZooKeeperShape. Tutorial has 14 spawns whose home area is missing from the
level: `Assault_HomeArea` ×4, `Charge_ZooKeeper` ×3, `RockThrow_Zookeeper`
×3, `FlameBreath_ZooKeeper` ×3 and `Lightining_HomeArea` ×1. These are shown
as unresolved, never matched to a similar name.

## Unconfirmed

- That the runtime picks exactly **one** linked group per WildlifeNode, and
  how it weighs the candidates. The data only shows 2–4 links per node.
- Whether a spawn uses any point inside `fRadius` or only the linked points.
  The editor draws the circle and the member links.
- Which external layer each game mode loads (main, mission, Large, DD).
- The purple zone colour comes from a reference screenshot of another tool,
  not from game data.
- How DynamicSpawnRules weighs keywords such as `NoElites`. The editor
  stores keywords verbatim.

## What the Level Editor does

**Viewport (Wildlife Spawns toggle).**

| Drawn as | Data |
| --- | --- |
| Marker meshes | Spawn points, groups and nodes |
| Green ring | Each group's `fRadius` |
| Faint green lines | Group → member point |
| Yellow lines | Node → candidate group |
| Yellow triangle | Joins the candidates of a set of three or more |
| Purple outline, hatched fill | Dynamic zone (terrain-draped when `ZooKeeperUseTerrainHeight=1`) |
| Blue lines | CreatureSpawn → home area |
| Red outline | Gameplay `AreaShape` boundary |

Overlays follow a move or vertex edit in progress.

**Outliner.** The Wildlife group has an All / Points / Sets / Circles / Zones
picker that filters the list. Each row names its type with the species,
radius, candidate count or keywords.

**Edits (Modding project only).**

| Edit | Path |
| --- | --- |
| Move, copy, duplicate and delete points, groups, nodes and zones; zone vertices | Existing draft path, with its undo |
| Species, radius, node category and enabled flag, zone keywords and the two zone flags (**Save wildlife settings**) | `LayerAuthoringEdit::Wildlife` |
| A node's candidates or a group's members (**Save links**) | `LayerAuthoringEdit::Links`, 1–16 targets of the correct class in the same mission file, by name or EntityId |

Both settings and links edits go through the committed project save, with a
verified backup and committed undo. The writer re-parses its output and
refuses the save when the values do not read back exactly. Deleting an entity
that another entity links to is refused.

Not yet supported: loading the ExternalLayers `WildlifeSpawns*.xml` files.
Maps whose spawns live only there show their zones but no points until those
layers are loaded, which needs EntityId collision handling against the
mission. Per-map opt-in packaging, so Legacy players keep the original routes,
is also not done yet.
