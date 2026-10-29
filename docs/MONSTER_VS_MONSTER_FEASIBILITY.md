# Monster-vs-Monster feasibility (offline Stage 2)

**Status:** feasibility assessment only. No Monster-vs-Monster experiment was run for
this document. The evidence is read-only inspection of the installed game, its
existing logs, and extracted asset copies. No game file, memory, or repository file
other than this document was changed.

**Scope:** offline, local play against the stock Stage 2 rules and assets. Online
play, dedicated servers, matchmaking, and anti-cheat compatibility are out of scope.
The local logs come from one user's installation and are not a reproducible test
fixture.

**Bottom line:** No **verified rules-, roster-, or asset-only path** to
Monster-vs-Monster (MvM) was found. The observed offline Hunt
configuration exposes one monster player slot, one monster spawn on the
inspected map, an AI faction table that treats monsters as friendly to one another,
a merc-centric win condition, and AI behavior built around hunting mercs. A real
two-player MvM prototype needs a verified way to create and team a second monster
player and to make monsters hostile to each other. Level assets could provide
another spawn, and configuration could change AI factions, but neither establishes
a second player slot or player-vs-player damage behavior on its own.

## Evidence roots

These are the local, read-only sources cited below.

| Alias | Path |
| --- | --- |
| `LOG-A` | `C:\Users\billn\AppData\Local\ModdedEvolveLauncher\logs\game\root-Game.log` |
| `LOG-B` | `C:\Games\ModdedEvolve\EvolveGame\LogBackups\Game.log` (archived rotating-log backup, 2026-10-29 01:51) |
| `AL` | `C:\Users\billn\AppData\Local\EvolveModManager\Data\AssetLibrary` |
| `FOREST-HUNT` | `AL\Level_Forest_Cliffs_data.pak__0a495253d54935c23783\files\Levels\Forest_Cliffs\ExternalLayers\Campaign\Hunt.xml` |
| `UI-TEXT` | `AL\USA-EN_xml.pak__03d946e18f5cb09c0e81\files\text_ui_menus.xml` |

The Stage 2 rules are under `AL\libs.pak__7ae536ff94b4e05646d5\files\libs\GameRules\`.
The Stage 2 AI files are under
`AL\scripts.pak__c731d6c975e1eaea1ca4\files\scripts\AI\`.
The separate 2012 readable Lua build is under
`AL\Scripts.pak__897f46b610c8e2022159\files\Scripts\`.

The live `C:\Games\ModdedEvolve\EvolveGame\Game.log` was rotated during this
inspection, so `LOG-B` cites the archived backup that preserves the earlier
sessions. `LOG-A` is a separate launcher-side log from the same installation.

## 1. What ships today

### 1.1 Roster: the characters exist; a second monster player was not observed

Eight playable monsters are present:

| Display name | Stage 2 `actorparams` key |
| --- | --- |
| Goliath | `GoliathA` |
| Meteor Goliath | `GoliathMeteor` |
| Kraken | `GrounderA` |
| Elder Kraken | `KrakenElder` |
| Wraith | `TeleporterA` |
| Behemoth | `RockyA` |
| Glacial Behemoth | `RockyBobcicle` |
| Gorgon | `LurkerA` |

Evidence: the display names and monster roster are in
`assets/discord/README.md:123-130`; the actor keys are the matching filenames under
`AL\libs.pak__7ae536ff94b4e05646d5\files\libs\Actor\Monsters\`
(for example `GoliathA.actorparams` and `RockyBobcicle.actorparams`). The
unreleased Necro Lazarus is explicitly excluded from the playable roster at
`assets/discord/README.md:40`. The logged role screen also reports eight monster
choices: `LOG-A:384` says
`class monster has 8 exciting characters to choose from`, and `LOG-A:383` preselects
`monster,3` for the local player.

This matters because roster availability is **not** the first blocker. The game
already knows about all eight monsters. The first unverified gate is a second
monster **player slot and opposing team**, not another character definition.

### 1.2 Lobby and player-slot model: one monster slot, four merc slots

Both local logs show the same offline Hunt arrangement.

- The session is offline single-player: `LOG-A:267` and `LOG-A:288` report
  `SinglePlayer`; `LOG-B` shows the same session shape.
- The selected rules are Hunt: `LOG-A:337` reports `Multiplayer = 0`, `Lobby Mode = 33`,
  `Num players = 1`; `LOG-A:343` reports `Game mode: 33 - Scenario: Hunt`.
  `LOG-B:275` and `LOG-B:281` repeat the same values in a separate run.
- The server starts with one player: `LOG-A:478` reports
  `MaxPlayers 0 -> 1`; `LOG-B:414` repeats it.
- The game allocates one merc team and one monster team:
  `LOG-A:2166` `team_merc[TRSTeam]`, `LOG-A:2167` `team_monster[TRSTeam]`;
  `LOG-B:2123-2124` repeats this.
- The observed player-slot set is: `PlayerSlot_spectator_00`,
  `PlayerSlot_merc_00` through `PlayerSlot_merc_03`, and exactly one
  `PlayerSlot_monster_00` (`LOG-A:2179-2184`; `LOG-B:2136-2141`).
- The local player is placed in that one monster slot: `LOG-A:2239` and
  `LOG-B:2196` report `userSlot: PlayerSlot_monster_00`.
- The end-of-round summary confirms the team assignment:
  `LOG-A:3303` reports `Class = Monster, Subtype RockyBobcicle, Team = 3`.

The initial team setting is `teamID: 0` (`LOG-A:353`, `LOG-B:291`), then the
pre-round role selection changes it to `teamID: 3` (`LOG-A:373`, `LOG-B:311`).
The logs prove the local player becomes the monster. They do **not** expose a
second monster player slot. No `PlayerSlot_monster_01` appears in either inspected
log. The backup contains later Hunt sessions with the same one-monster layout
(for example `LOG-B:10228-10234`, `LOG-B:12032`, `LOG-B:19304-19309`).

**Inference:** adding another local player as a monster is not a matter of choosing
a different character in the existing menu. The lobby's player-slot set has no
second monster slot to receive that player.

### 1.3 Team and faction model: monsters are friendly to monsters

`AL\scripts.pak__c731d6c975e1eaea1ca4\files\scripts\AI\Factions.xml:2` contains
the AI faction reaction table. The relevant reactions are:

- `Mercs -> Monsters: hostile`
- `Monsters -> Mercs: hostile`
- `Monsters -> Monsters: friendly`
- `Players -> Monsters: hostile`

So the shipped AI faction model does not merely lack monster-vs-monster hostility;
it explicitly makes monsters friendly to each other. A second monster would need a
different team/faction relationship, or a special friendly-fire rule, before it
could be treated as an opponent. Whether player damage uses this same table or a
separate team-damage check is not established here (see Section 5).

The AI files reinforce the same assumption:

- `AL\scripts.pak__c731d6c975e1eaea1ca4\files\scripts\AI\SelectionTrees\BTGoliathHuntPrimary.xml:2`
  and `BTRockyHuntPrimary.xml:2` contain `ChargeNearestMerc`,
  `TargetDistToClosestMerc`, and `AliveMercs`. Their target logic is written around
  mercs.
- `AL\scripts.pak__c731d6c975e1eaea1ca4\files\scripts\AI\SelectionTrees\BTTeam_Monster.xml:2`
  is an `IdleForTime` wait tree, not a monster-vs-monster combat tree.

For player-vs-player MvM, the human player does not use those AI trees. The faction
table still shows that the shipped hostility model is built around mercs; the
selection trees are an additional blocker for any AI-assisted or bot variant.

### 1.4 Spawn model: the inspected Hunt layer has one monster spawn point

`FOREST-HUNT:33` defines exactly one Hunt monster spawn:

```xml
<Entity Name="SpawnPoint_Hunt_Monster_01" ... EntityClass="SpawnPoint" ... Layer="Hunt">
    <Properties bEnabled="1" teamName="monster"/>
</Entity>
```

A read-only search of that file finds one occurrence of
`SpawnPoint_Hunt_Monster_01` and no `SpawnPoint_Hunt_Monster_02`.
This is one concrete level, not proof about every level, but it is the level used
in the inspected logs. It shows that a second monster slot would also need a
second spawn location, or a deliberate shared/duplicate spawn policy. The existing
Hunt layer does not provide one on Forest Cliffs.

Nest uses a different model: `AL\libs.pak__7ae536ff94b4e05646d5\files\libs\GameRules\Nest.xml:1`
defines `EggHatching`, `SelectableMonsters`, and `nMaxMinions=1`. That is an
AI-monster objective/minion system, not a second player-controlled monster slot.

### 1.5 Win conditions and mode rules: merc-centric or non-terminal

The main Hunt rules are:

- `AL\libs.pak__7ae536ff94b4e05646d5\files\libs\GameRules\Hunt.xml:1`:
  `TimelimitWinner="merc"`, `MercsLoseWithNoGenerators="true"`,
  `GeneratorMonsterStage="3"`, and `RoundDuration="12:00"`.
- `AL\libs.pak__7ae536ff94b4e05646d5\files\libs\GameRules\ScenarioData.xml:1`
  defines a single-monster progression model:
  `fMaxEnergyStage1="24"`, `fMaxEnergyStage2="28"`,
  `fMaxEnergyStage3="-1"`, and `nInitialMonsterStage=1`.
- `AL\libs.pak__7ae536ff94b4e05646d5\files\libs\GameRules\ArenaMode.xml:1`
  has `nBattleWinsRequired="2"`, `DisableMercAliveCheck="true"`,
  `DisableMonsterAliveCheck="true"`, and `TimelimitWinner="spectator"`.
  It is an arena ruleset, but it contains no monster-vs-monster team rule.
- `AL\libs.pak__7ae536ff94b4e05646d5\files\libs\GameRules\Training.xml:1`
  also disables both alive checks, sets `RoundDuration="0:00"`, and starts the
  monster at stage 3. It is the closest existing **sandbox** ruleset, but it still
  has one monster player slot and no symmetric win condition.
- `AL\libs.pak__7ae536ff94b4e05646d5\files\libs\GameRules\Defend.xml:1`
  is an AI-defense scenario with `SpawnCap=3` and a wave/objective structure, not
  a symmetric duel.
- `AL\libs.pak__7ae536ff94b4e05646d5\files\libs\GameRules\CustomGameOptions.xml:1`
  exposes difficulty, round length, handicap, strikes, perks, wildlife, weather,
  and Arena options. It exposes no monster count, monster team, or MvM toggle.

Thus even the most permissive shipped ruleset (Training) does not end a match by
"last monster standing". It suppresses round end; it does not define a new one.

### 1.6 UI and social model: no MvM entry point in the inspected UI

`docs/SELECTION_ROLE_PROTOTYPE.md:6-11` documents that the current role prototype
shows Hunter and Monster (plus Observer) and retains the native `SelectClass` call,
but is an **interaction prototype** that has not been observed in-game. The same
document lists missing in-game verification of ability, perk, purchase, skin,
readiness, and callback paths at `docs/SELECTION_ROLE_PROTOTYPE.md:43-48`.

The shipped UI text contains direct warnings about the one-monster model:

- `UI-TEXT:1` defines `ui_warning_no_parties_for_monsters`:
  "To queue as the Monster, you must first disband your party."
- `UI-TEXT:1` defines `ui_warning_cannot_join_monster`:
  "Unable to join game. The Monster player is unable to invite other players."

A read-only search of the inspected `UI-TEXT` file found no
`Monster-vs-Monster` or `Duel` string and no `local multiplayer` string. That is
evidence about this localization file, not proof that no engine code could be
patched. It does mean there is no shipped menu entry to select for MvM.

### 1.7 Earlier readable Lua confirms the concept, not current behavior

The Stage 2
`AL\scripts.pak__c731d6c975e1eaea1ca4\files\scripts\GameRules\TrsMultiplayer.lua`
and `Campaign.lua` files are compiled Lua bytecode. Only their string tables are
readable. The Stage 2 `TrsMultiplayer.lua` string table contains
`AllowMercPlayers`, `AllowMonsterPlayers`, and `DefaultMonsterSpawnAreas`; the
Stage 2 `Campaign.lua` string table contains `campaign_playable_monsters`. This
shows the team/player-allowance concepts still exist in the build, but it does
**not** prove that a second monster slot can be allocated.

The separate 2012 build is plaintext and is useful only as design history:

- `AL\Scripts.pak__897f46b610c8e2022159\files\Scripts\GameRules\TrsMultiplayer.lua:42`
  has `playableTeamName={ "merc", "monster" };`.
- Same file, line 46: `DefaultMonsterSpawnAreas = "MonsterSpawn";`.
- Same file, lines 74-75: `AllowMercPlayers = false;`
  `AllowMonsterPlayers = false;`.
- Same file, lines 78-79: `MercTeamWipeEndsRound = true;`
  `MonsterTeamWipeEndsRound = true;`.
- Same file, line 619: `GetDefaultTeam` passes
  `AllowMercPlayers, AllowMonsterPlayers` to the game.
- `AL\Scripts.pak__897f46b610c8e2022159\files\Scripts\GameRules\Campaign.lua:16-19`
  sets `AllowMercBots = true`, `AllowMercPlayers = true`,
  `AllowMonsterPlayers = false`, and `UseMinPlayerLimit = true`.
- Same file, lines 32-39 expose `GetMaxMercs()` and `GetMaxMonsters()` from
  `campaign_playable_mercs` and `campaign_playable_monsters`; lines 75-79 derive
  the allow flags from those cvars.

The 2012 code shows a two-team model with a configurable player allowance. It does
not show two monster **slots**, two monster **teams**, or monster-vs-monster
hostility. Its team-wipe rules are merc-team and monster-team checks, not two
opposing monster teams. Treating this as proof of Stage 2 behavior would overstate
the evidence.

## 2. Methods ranked by feasibility

No method below is currently a supported MvM path. The ranking is about how much
each approach can prove per unit of risk, not about whether it delivers the final
feature.

| Rank | Method | What it would change | Feasibility | Assessment |
| --- | --- | --- | --- | --- |
| 1 | Read-only slot/team proof | Nothing; collect stock logs, lobby dumps, rules, and spawn data | **High as diagnosis** | Already sufficient to show the shipped one-monster model. Must be the first gate. |
| 2 | Game-mode rules sandbox | A disposable Training/Hunt clone with round-end and alive checks relaxed | **Low to medium** | Rules alone do not establish a second monster slot or an additional level spawn. Only useful after a slot exists. |
| 3 | Runtime roster/role changes | Role-select/team assignment and a second monster player slot | **Low** | Roster and UI already expose monsters; the missing player slot and team are runtime/engine concerns, not asset fields. |
| 4 | AI/config-only hostile monster | Faction reactions, AI target trees, an AI monster spawn | **Low for PvP; low-to-medium for PvE** | Could produce a monster-vs-AI-monster sandbox only after AI target-tree changes; it is not two human players and not a symmetric objective. |
| 5 | Asset/skin-only change | Models, textures, portraits, cosmetics | **Not applicable** | Does not change slots, teams, spawns, hostility, or win conditions. It is not an MvM method. |

### 2.1 Why the rules path is not enough

A rules-only experiment is attractive because it is the most reversible. It can
make a round non-terminal, disable merc/monster alive checks, and change timers.
But the inspected rules and logs show three independent gaps that rules do not
close:

1. **No observed second monster player slot.** `PlayerSlot_monster_00` exists;
   no `PlayerSlot_monster_01` is allocated in the inspected sessions.
2. **No second Hunt monster spawn on the inspected map.** Forest Cliffs Hunt has one
   `SpawnPoint_Hunt_Monster_01`; a level-asset edit could add another if needed.
3. **No shipped monster hostility or symmetric win check.** The AI faction table has
   `Monsters -> Monsters` as `friendly`, and the win conditions are merc
   timelimit/generator rules or a non-terminal Training sandbox.

A rules PAK can change the third item's *symptoms* (for example, disable a round
end), but does not itself establish a second player slot or a second map spawn.
The spawn is potentially addressable in level assets; the player slot remains the
decisive unresolved gate.

### 2.2 Why the roster/role path is not enough

The role prototype documents a UI that can present a monster class and call the
native selection path, but it does not prove a second monster *player*. The logs
show the local player already selects the monster successfully (`LOG-A:383-384`) and is
assigned `Team = 3` (`LOG-A:3303`). The failure is downstream: the lobby allocates
one monster slot and one monster team. Roster changes therefore address the part
that already works, not the gating part.

### 2.3 Asset/config-only is not justified for MvM

Cosmetic assets cannot create player slots, team relationships, damage rules, or
round-end behavior. **Level assets can change spawn points**, and AI faction
configuration can change AI relationships, but neither establishes a second
monster player or verifies player-vs-player damage. Such edits should be deferred
until a runtime prototype proves that two monsters can exist as opponents.

## 3. Blockers

| Blocker | Shipped evidence | Consequence |
| --- | --- | --- |
| **Team assignment** | `Factions.xml:2` has `Monsters -> Monsters:friendly`; logs show one `team_monster` and `Team = 3`. | The shipped AI faction model treats a second monster as an ally; a second team or an explicit hostile override is required. |
| **Player slot** | `LOG-A:2179-2184` and `LOG-B:2136-2141` allocate one `PlayerSlot_monster_00` and no `PlayerSlot_monster_01`. | There is no lobby destination for a second monster player. |
| **Spawn** | `FOREST-HUNT:33` has one `SpawnPoint_Hunt_Monster_01`. | Even with a slot, the Hunt layer has one monster start location on the inspected map. |
| **UI** | `docs/SELECTION_ROLE_PROTOTYPE.md:6-11`, `43-48`; `UI-TEXT:1` warnings about monster parties/invites. | No verified two-monster selection, party, lobby, HUD, or end-of-round flow. |
| **AI** | `BTGoliathHuntPrimary.xml:2` and `BTRockyHuntPrimary.xml:2` use merc targets; `BTTeam_Monster.xml:2` is an idle tree. | AI monsters are not prepared to fight another monster; a bot-assisted variant needs new targeting/faction behavior. |
| **Win condition** | `Hunt.xml:1` (`TimelimitWinner="merc"`, generator condition); `ScenarioData.xml:1` (single-monster stages); `Training.xml:1` (no round duration); `ArenaMode.xml:1` (spectator timelimit). | No shipped rule says "last monster standing" or scores two monster teams. |
| **Network / launch** | `docs/DIRECT_GAME_LAUNCH_EXPERIMENT.md:69-71` says native process creation does not reproduce client services; offline tests are the only defensible scope. | Do not present MvM as an online or matchmaking feature. A two-client test would add service/team-session problems on top of the slot problem. |
| **Authentication / swap** | `docs/OFFICIAL_RUNTIME_SWAP_DECISION.md:11-19` and `docs/DIRECT_GAME_LAUNCH_EXPERIMENT.md:63-67` require reversible staging, journals, and restoration. | Any experiment must stay in an isolated test installation; manual persistent replacement is not acceptable evidence. |

The roster is not a blocker. The eight monsters are present, and the role screen
already reports eight choices. Perks, abilities, and cosmetics matter for polish
after the slot/team gate, not for the initial feasibility decision.

## 4. Read-only and reversible validation plan

This plan is deliberately staged. Stop at the first failed gate. Do not bypass a
failed authentication check, unresolved original backup, or active recovery
journal.

### Phase 0 — read-only baseline (no game changes)

1. Record the selected **Test Game** root and the exact build/stage being tested.
   Keep production and test installations separate, as described in
   `docs/DIRECT_GAME_LAUNCH_EXPERIMENT.md:4-25`.
2. Hash the original files that would be involved in a later experiment and retain
   the manager's journal. Do not edit or replace anything for this phase.
3. Launch stock offline Hunt once and capture the log. Confirm:
   - `SinglePlayer`;
   - `Lobby Mode = 33` / `Scenario: Hunt`;
   - `MaxPlayers 0 -> 1`;
   - `PlayerSlot_merc_00..03` and exactly one `PlayerSlot_monster_00`;
   - no `PlayerSlot_monster_01`;
   - the local player's `userSlot: PlayerSlot_monster_00`;
   - one `SpawnPoint_Hunt_Monster_01` in the map layer.
4. Parse `Factions.xml`, `Hunt.xml`, `ScenarioData.xml`, `Training.xml`, and
   `ArenaMode.xml` read-only. Record the exact faction reaction, timelimit winner,
   alive-check flags, and monster-stage values.
5. Run the manager's read-only original-PAK audit if the installation supports it
   (`docs/SELECTION_ROLE_PROTOTYPE.md:59-64`). Keep its report with the baseline.
   Use the launch/restore checks in `docs/DIRECT_GAME_LAUNCH_EXPERIMENT.md:81-86`
   when a later phase actually launches the game.

**Stop condition:** if there is no second monster slot in the stock session and no
second Hunt spawn on the inspected map, do not build a rules or UI prototype yet.
The next step would be engine/runtime work, not asset work.

### Phase 1 — reversible rules sandbox (only if a slot proof exists)

1. Use an isolated Test Game signing stage. Build only in scratch; do not copy a
   PAK into the live `Game/` directory.
2. Change the smallest possible rules surface in a disposable Training/Hunt clone:
   no round end, no merc/monster alive checks, and a long or unlimited timer.
   Preserve the original rules for comparison.
3. Use the manager's reversible swap and watchdog path, not a manual persistent
   install. The required properties are documented at
   `docs/DIRECT_GAME_LAUNCH_EXPERIMENT.md:63-67`: validate the mapping and digests,
   record `swapping`/`swapped`, restore originals on exit, and preserve backups if
   recovery fails.
4. Confirm in the log that originals are restored and the journal is inactive after
   exit. Re-hash the touched originals and compare with Phase 0.
5. Treat a successful launch as **only** a rules test. It is not evidence of a
   second monster, hostility, or a win condition.

### Phase 2 — runtime slot/team prototype (separate engineering task)

This phase is not an asset or rules change. It requires a runtime/engine change and
should not be promised as a mod-profile feature until it passes:

1. Allocate a second monster player slot (for example a `PlayerSlot_monster_01`)
   in an isolated session.
2. Assign the second monster to an opposing team or an explicit hostile faction
   relationship.
3. Provide a second valid spawn on the test map, or define and verify a shared
   spawn policy.
4. Verify that both monsters can damage each other, that abilities/targeting/HUD
   behave, and that the round ends on a defined condition.
5. Verify that all originals are restored and no journal remains after exit.

**Abort criteria for every phase:** authentication failure, missing original,
unresolved backup, active journal, crash on load, or a log that shows only one
monster slot. Abort and restore; do not retry over a dirty state.

## 5. Explicit unknowns

These are not resolved by the inspected evidence:

- Whether the Stage 2 engine can allocate a second **player-controlled** monster
  slot at runtime without a native code change.
- Whether a second local player can join an offline session at all, given the
  `SinglePlayer`/`MaxPlayers 0 -> 1` lobby behavior in the logs.
- Whether `Monsters -> Monsters:friendly` can be overridden by team assignment
  alone, or whether damage/friendly-fire code also needs a change.
- Whether the AI-only `Factions.xml` reactions govern player-vs-player damage, or
  whether player teams use a separate damage relationship not visible in that file.
- Whether monster abilities, targeting, status effects, and HUD elements are
  correct when the target is another monster rather than a merc.
- Whether monster stage/energy progression is per-player or a single global
  resource when two monsters are present.
- Whether the AI can be retargeted to a monster with faction/AI-tree edits, and
  whether that would be sufficient for a bot-assisted variant.
- Whether any hidden console variable or developer mode creates a second monster
  slot. The inspected logs and UI strings do not show one, but absence of evidence
  is not proof of absence.
- Whether Arena's battle/round machinery can be repurposed into a symmetric duel
  without new code. Its shipped settings are spectator-timelimit plus merc/monster
  alive-check flags, not a two-monster score model.
- Whether the 2012 Lua design maps cleanly to Stage 2. The Stage 2 scripts are
  compiled; only strings and behavior observed at runtime are reliable.
- Whether a two-client local test can avoid the client-service limitations
  described in `docs/DIRECT_GAME_LAUNCH_EXPERIMENT.md:69-71`.

## 6. Recommendation

**Do not ship or advertise Monster-vs-Monster as a rules/config/asset mod.** The
inspected stock setup does not establish a working low-risk path:

- the roster is already present;
- the observed player-slot set has one monster;
- Hunt has one monster spawn on the inspected map;
- the AI faction model treats monsters as friendly to one another;
- the AI hunts mercs;
- the win conditions are merc-centric or deliberately non-terminal; and
- no MvM UI entry point appears in the inspected localization.

If the feature is still desired, the next unit of work should be a narrowly scoped
**runtime slot proof** in a disposable offline installation. That proof should
answer one question: can the game create a second monster player and place it on an
opposing team? If the answer is no, rules and asset work cannot recover the feature.
If the answer is yes, a reversible Training-based sandbox can follow, then a
purpose-built spawn/round-end/UI prototype. Online support should remain out of
scope until the offline prototype is stable and independently verified.

A monster-vs-AI-monster sandbox using Nest/Defend-style AI monsters is a separate,
lower-fidelity idea. It may be technically easier, but it is not player-vs-player
MvM and should be labeled as a PvE experiment, not as the requested feature.
