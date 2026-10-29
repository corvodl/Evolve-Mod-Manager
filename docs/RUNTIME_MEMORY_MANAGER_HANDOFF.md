# Offline Stage 2 runtime research: manager implementation handoff

**Scope and status.** This is an evidence handoff, not an implemented feature or
authorization to patch executable code. Results came from read-only inspection of
one installed Stage 2 build and a few explicitly guarded, restored **data-field**
experiments in offline matches. No native item/slot method was invoked. All RVAs,
vtable shapes, and offsets below are **build-specific research observations**, not
portable signatures. Match entity IDs, actor addresses, and inventory addresses
change across loads; never persist them as configuration.

Related: `docs/MONSTER_VS_MONSTER_FEASIBILITY.md` (PR #133) covers mode/asset
evidence; `docs/SLOT_ALLOCATION_EVIDENCE.md` contains the slot/team disassembly
and live-read trace. The earlier effectset and perk research is in PR #127 and
`docs/perk-editing-and-match-inspection.md`. This handoff describes what a future
**manager implementation PR** could safely expose, and where it must stop.

## Evidence and confidence

| Area | Observation | Evidence / limit |
| --- | --- | --- |
| Process and identity | External reads resolved the local entity ID, entity pointer, actor pointer (actor vtable RVA `0x24B3790`), entity backlink `actor+0x38`, and movement link `actor+0x240`. Per-entity two-hop searches resolved five actor objects in an earlier Goliath match. | Temporary `probe_match_actor_ids.py`; `crates/evolve-desktop/src/runtime_actors.rs` already implements guarded actor inspection. Validate the full chain on every inspection; the game reloaded during research and invalidated old IDs. |
| Current roster example | Following a restart, process PID `64692` had **Griffin** (`TrackingABot`, local ID `0x17527`), not Abe, and Hyde (`AssaultDBot`, `0x1751A`). Griffin's and Hyde's inventories each contained 11 item IDs; Hyde had live `FlameThrowerB`, `HeavyMachineGunB`, and `NoxiousGrenade` items. | `Game.log` roster at approximately 01:53:50 and a guarded live read. **Historical example only**; never hardcode these IDs/pointers or infer the player's character from the previous match. |
| Gravity | Physics-world `p_gravity_z` at `+0x128` read `-13.0`; match-character `CLivingEntity+0x1C0` read `-9.81`. | Read-only snapshots; neither target was write-validated. They are distinct values, not interchangeable gravity controls. |
| Projectile velocity | Lightning Balls: physics vtable RVA `0x2338150`, velocity vector `+0x2E0`. Abe grenade and dart ammo, Emet homing shots, Kraken Plasma Blasts: physics vtable RVA `0x2336380`, vector `+0x1E0`. Goliath Rock Throw shared the first vtable but **did not** use its observed `+0x2E0` as flight velocity. | Temporary `abe-projectiles/`, `rock-extended/` captures and `analyze_projectile_captures.py`; identify class and behavior before interpreting a vector. No speed write was validated. |
| Jetpack | On Abe, `actor+0xE68` tracked four sustained climb periods as a thrust-state candidate. The earlier `+0x5BC` fuel hypothesis was ruled out. | `abe-jetpack-controlled/meta.json` and capture script; not a validated fuel meter or write target. |
| Inventory | Actor `+0x170` pointed to inventory (vtable RVA `0x2263350`); inventory owner ID `+0x20`, entity backlink `+0x38`; item-ID array `[+0x40,+0x48)`; 16-entry loadout table of `{u32 item ID, u32 flag}` starts at `+0x6C`; current ID `+0xF0`, previous ID `+0xF8`; actor `+0x2F0` and `+0x14E0` mirrored current selection. | `abe-inventory-selection/meta.json` (850 frames) and read-only inventory disassembly. Six observed equip transitions copied previous current to `+0xF8`, with no loadout-table changes and no mirror mismatches. Item IDs are **not** slots or pointers. |
| Failed swap experiments | A guarded write to Abe's actor equipped ID was overwritten within 50 ms. Substituting another hunter's live Lightning Gun ID into Abe's grenade item-list entry hid that slot rather than providing a usable gun. Writes were restored. | Temporary `test_abe_inventory_slot.py` and earlier guarded actor-field probe. **Do not ship an item-ID swap** as a transfer/equip implementation. |
| Equip candidate | A live-code method at RVA `0x446E88`, secondary vtable RVA `0x2264460` entry `[35]`, appears to resolve a new item, deactivate the old one, set previous/current inventory IDs, notify the actor, and activate the new item. | Read-only disassembly and observed transition shape; no call or ownership-transfer test. It is **not** a validated public ABI, safe thread entry, or cross-owner swap routine. |
| Item-system API hypothesis | Evolve 2012 scripts call `ItemSystem.GiveItem(className, actorId)` and `GiveItemPack(actorId, pack, keepOld, selectPrimary)`; upstream CryEngine exposes `GiveItem`, `SetActorItem` and `IItem::PickUp`. Stage 2 logs show Lua running but do not prove that the same bindings or ABI are callable. | Historical 2012 `Scripts.pak` (`Scripts/Entities/Others/ItemSpawner.lua:198`, `Scripts/Entities/actor/player.lua:1894`); upstream `IItemSystem.h`. Evolve's bindings/ABI diverged from upstream. Never guess vtable indices from the upstream header. |
| Command reach | The Stage 2 engine logs internal `[CONSOLE]` execution and rejects cheat CVars. The user could not open a retail console; no installed overlay/DLL command/Lua bridge was verified. | `Game.log`, `LogBackups/Game.log`, and installed module/launcher inspection. Upstream `~`, `-devmode`, `+cmd`, and `#` Lua syntax are **not** verified Stage 2 access paths. No console/config change or injection was attempted. |
| Monster player slots | On the observed offline Hunt initialization path, desired monster slots are overwritten with the literal `1` (RVA `0x15CF827`); merc slots with `4` (`0x15CF820`). The separate team-3 admission getter (`0x1124240`) returns only `0` or `1`; `SetTeam` (`0x112DD30`) rejects a join at capacity. The observed log has `monster_00`, not `monster_01`. | `docs/SLOT_ALLOCATION_EVIDENCE.md` documents the control flow, live settings and limitations. Key instructions were independently reread from the decrypted live image. A second spawn can be added via level assets, but does not resolve these native limits. |

## Implementation boundaries and suggested PR sequence

1. **Read-only, build-gated diagnostics first.** Add a typed runtime snapshot
   (PID, executable path/hash, module base, match epoch, entity ID, actor/entity
   backlinks, inventory owner and array bounds, slot/roster log provenance). Resolve
   from the currently running process each time; mark stale/unverified values rather
   than reuse them. Expose distinct labels for entity ID, item entity ID, actor
   pointer, inventory item-list entry, equipped item ID, and player slot.
2. **Read-only developer panels next.** Show per-character gravity observations,
   jetpack-state candidate, class-validated projectile velocity, item ownership,
   and observed monster slot/team limits, each with confidence/build status. Avoid
   treating shared physics vtables as proof of the same projectile layout. Include
   synthetic tests for bounds, stale PID/epoch, changed vtables, mismatched entity
   backlinks, and a loadout/current-ID mismatch.
3. **Separate, explicit offline write proposals.** Existing Developer controls
   already use process validation and compare-before-write for known fields
   (`runtime_actors.rs`, `runtime_live.rs`). Gravity, velocity and jetpack fields
   above are **not** write-validated. A future experiment needs a known local
   offline session, exact build identity, fresh actor/item identity, original-byte
   journal, bounded change, observation, and conditional restore; no automatic
   feature should be enabled from these readings alone.
4. **Item transfer/equip stays blocked.** Require a verified Stage 2 engine-owned
   give/pickup/transfer path, correct owner and item registration, game-thread or
   authoritative server execution, and an observed result (item exists in the
   recipient's inventory, selected weapon/animation works, original owner state
   remains coherent). Do not call RVA `0x446E88` as a shortcut; do not substitute
   a foreign entity ID into inventory, mirrors, or loadout entries. If an item is
   absent from the match, merely changing an ID cannot create/register it.
5. **Two-player Monster vs. Monster stays research-only.** The first gate is an
   additional player slot **and** admission/ownership for a second monster player;
   a second actor or spawn alone does not prove it. Validate human-controlled
   actor initialization, team hostility/damage, camera, HUD, spawn, and round end
   only after the slot/admission gate. No ruleset/UI or gameplay PAK edit is part
   of the proposed initial runtime experiment. Do not present the observed Hunt
   limit as a proof about every mode or online session.

## Where a manager PR would integrate

- `crates/evolve-desktop/src/runtime_memory.rs`: process snapshot and read-only
  address resolution. Add per-build identity and snapshot invalidation here, not
  a persistent actor-address preference.
- `crates/evolve-desktop/src/runtime_actors.rs`: existing guarded actor discovery;
  extend typed inspection rather than bypassing its backlink/PID checks.
- `crates/evolve-desktop/src/runtime_match_perks.rs`: already parses the latest
  `PlayerSlot_merc_*` / `PlayerSlot_monster_*` roster; preserve the distinction
  between a logged slot and a validated live actor.
- `crates/evolve-desktop/src/runtime_experiments.rs`: equip/spawn are **previews**;
  keep them non-executable until all prerequisites above are proved.
- `crates/evolve-desktop/src/main.rs`: password-gated Developer state and
  asynchronous inspection/preview events; do not route these experiments into
  ordinary launch or matchmaking paths.

**Review/acceptance checklist:** no writes in the diagnostic PR; per-build guard
rejects unknown images; stale PID/world/actor/owner fails closed; test fixtures
cover ID-vs-pointer and selected-vs-registered-item distinctions; UI reports
read-only vs observed reversible experiment vs unverified hypothesis plainly;
feature flags for item transfer and MvM stay off. Subsequent write-capable work
requires separate review and explicit offline authorization.

The temp Python captures and logs under
`C:\Users\billn\AppData\Local\Temp\opencode\` and local game/AssetLibrary paths
are **investigation artifacts, not bundled test fixtures**. Reproduce with fresh
read-only captures and verify executable hash before depending on any address.
