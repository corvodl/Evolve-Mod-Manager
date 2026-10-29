# Perk editing and match inspection

## Edit `.effectset` values (offline only)

Import the game's source PAK into the Asset Library, create an editable project,
and open **Gameplay values…**. Search for a perk under
`Libs/Items/Perks/*.effectset`, then filter its fields by `value`. The form
accepts authored text XML and CryXmlB, preserves the original document as a
backup, and keeps pipe-separated variant counts unchanged. Edits to effect
sets, including files under `Libs/EffectSets/`, are classified as gameplay
changes and must be tested offline. Imported source archives are not edited.

For example, `MPerkSavageNatureHighEffect.effectset` defines separate climb,
feeding, smell and auto-sense modifiers. Do not assume that the three in-game
perk slots share the same value or that modifying the source definition changes
an already-running match. Rebuild the project and start a new offline match to
test an edit; game acceptance and modifier stacking are not inferred by the
manager.

## Developer-menu readout

**Refresh actors and match perks** reads the running game's `Game.log`
round-start character/perk groups and compares their perk-name multiset with
currently spawned perk entities. It also requires the logged roster to match
the validated local actor. A failed comparison displays an error instead of
presenting last match's loadout as current. Actor IDs are only shown when an
actor name matches exactly one character slot. Repeated characters or unknown
actor names remain unassigned; the manager does not guess ownership from entity
creation order.

The readout shows selected perk *IDs*, not localized tooltip names, applied
bonuses, or writable offsets. It refreshes on demand and does not claim to
detect mid-match changes after the logged selection. The entity/name layout is
build-specific; unreadable or incompatible builds fail closed. Reading the
roster does not write game memory.
