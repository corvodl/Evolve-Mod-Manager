# Stage 2 Player-Slot Allocation & Monster Actor Init — Evidence (read-only)

Scope: offline 1-player Hunt (`Multiplayer = 0`, `Lobby Mode = 33`, `Num players = 1`, `Scenario: Hunt`)
on the local Stage 2 install. The investigation made **no game-file or process-memory
changes and no native calls**; temporary read-only probes and this document were created.

Method: `Evolve.exe` on disk is packed, but the live mapped image is decrypted, so all native evidence is
RIP-relative xref scanning + Capstone disassembly of the running image (PROCESS_VM_READ only), plus
read-only field dumps of live heap objects.

Image: `C:\Games\ModdedEvolve\EvolveGame\bin64_SteamRetail\Evolve.exe`, ImageBase `0x140000000`,
live base `0x7ff7362a0000` (PID 64692). All addresses below are **RVAs**.

---

## 1. Slot entities and naming (proven)

| # | Finding | Evidence |
|---|---------|----------|
| 1.1 | Slot name format is `PlayerSlot_%s_%2.2d` (team short name + 2-digit index) | string RVA `0x240EF50`; single xref at `0x15CF4B2` |
| 1.2 | Team short name comes from `GetTeamShortName(teamId, 0)` at `0x15CF4AA` (`call 0xCDB138`) | `0x15CF4A5`–`0x15CF4C0` |
| 1.3 | Per-team index loop is `0..15` (`cmp ebx, 0x10` / `jl`), i.e. decimal `_00`–`_15` are possible names | `0x15CF4DB`–`0x15CF4E0` |
| 1.4 | The loop builds the name for the **next free** index and stops at the first index that does **not** resolve; the resolved name is then passed to slot creation | `0x15CF4C5`–`0x15CF4E2` (`FindSlot` at vtable `+0x68`) |
| 1.5 | One slot object is created per call: `manager->vtable[0x50](params, 1)` with the class handle for `"PlayerSlot"` and the built name | `0x15CF542`–`0x15CF59A`; class string `0x240EAA0` |
| 1.6 | Post-create check compares the team's slot count against `desired[teamId]` and sets a "team full/ready" flag | `0x15CF626`–`0x15CF65F` (`cmp ebx,[rax+r15*4]`, `setl`) |
| 1.7 | The global used by the creation code (`[0x28D58E8]`, vtable `0x22DDD00`) is the entity/object factory, not a slot-specific manager: `+0x48` → class registry (`[this+0x40180]`), `+0x38("PlayerSlot")` → class handle, `+0x50(cls,name,params,1)` → create entity, `+0x68(name)` → name→id lookup (rb-tree `strcmp` on `node+0x20`, id at `node+0x28`), `+0x60(id)` → entity-by-id | `0x9D66BC` (name→id), `0x9D6960` (id→entity, `[this+0xA8]`), `0x9D6950` (`[this+0x40180]`) |
| 1.8 | Entity storage is a dynamically grown pointer array at factory `+0xA8`; during the live 02:37 match it already held **≥32** objects (ids 1..31 observed, same vtable `0x22DBC60`), so raw storage is *not* the 6-slot limiter | live read of `[[0x28D58E8]+0xA8]` at 02:39 |
| 1.9 | Each object stores its id at `+0x18` | `0x9D6994` (`cmp [rax+0x18], edx`) |

Runtime confirmation (one-player Hunt, `Game.log`):
`2099` spectator_00, `2100`–`2103` merc_00..03, `2104` monster_00 — **six** slots, no `monster_01`
(same set repeats at `9260`–`9265` for the second match).

---

## 2. How many slots get created (the hardcoded "1")

| # | Finding | Evidence |
|---|---------|----------|
| 2.1 | Slot creation is a single driver that only runs while the game's slot list is empty (`[this+8] == [this+0x10]`), i.e. at match/server init only — it cannot be topped up mid-match | `0x15CF67B`–`0x15CF683` |
| 2.2 | Driver front-end fills a 7-int "desired counts" struct via `0x15CF83C` | `0x15CF685`–`0x15CF68A` |
| 2.3 | Driver then loops `team = 0..6` and calls `CreateTeamSlot(team)` exactly `desired[team]` times | `0x15CF691`–`0x15CF6B2` |
| 2.4 | Desired-counts wrapper calls the real computation `0x15CF868`, then **overwrites two fields with literals** | `0x15CF83C`–`0x15CF865` |
| 2.5 | The literals: `desired[+8] = 4` (**merc**) and `desired[+0xC] = 1` (**monster**) | defaults table `0x15CF808`: `mov dword [rcx+8], 4` @ `0x15CF820`, `mov dword [rcx+0xC], 1` @ `0x15CF827`; copied at `0x15CF854`→`0x15CF859` |
| 2.6 | Only the **observer** count stays computed (`desired[+4]` from getter `0x112426C`) | `0x15CF8BA`–`0x15CF8BF` vs `0x15CF83C` override |
| 2.7 | Getter meanings were verified against the log format `"Desired Slot Counts - merc: %d  monster: %d  observer: %d"` (string `0x240EF10`): merc=`out[+8]`, monster=`out[+0xC]`, observer=`out[+4]` | `0x15CF6E2`–`0x15CF6F4` |

**Conclusion:** the observed initialization path overwrites the number of
`PlayerSlot_monster_*` entities with a literal **1** from the defaults table. No
configuration source has been identified that overrides this path. The observer count
uses a separate getter (`0x112426C`, which falls back to `sv_maxspectators`, registered with default `0` at
`0x6321E3`).

Computed values under the live settings object (read at `[[0x351FCF0]+0x1A0]+0x38`, field `+0xC798`
= `0x00000021`; `[0x351FD08]+0x440` = `1`): merc getter `0x112421C` → 4, monster getter `0x1124240`
→ 1, observer getter `0x112426C` → 1. That is exactly the 4/1/1 observed, and it independently
confirms the model; the log line itself never appears in any log file (channel not enabled).

The same read was repeated **during a live match** (02:39, third Hunt of the session) and returned the
identical `0xC798 = 0x21`, `[+0x440] = 1`, while `Game.log:18216`–`18221` shows the same six slots
(spectator_00, merc_00..03, monster_00) for that match — so the values are not a menu-only artefact.

---

## 3. Per-team membership capacity (second hardcoded 1)

`SetTeam` = RVA `0x112DD30`, signature `(this, teamId:edx, userHandle:r8, bForce:r9b)`.

| # | Finding | Evidence |
|---|---------|----------|
| 3.1 | Observer team is gated: if `teamId == 1` and `[0x351FD08]+0x440 == 0`, team is forced to 0 | `0x112DD5E`–`0x112DD78` |
| 3.2 | The player/server entry list is at `this+0x960`, stride `0xB0`; per-entry team id byte at `entry+0xAE` | `0x112DD4A` (`lea rsi,[rcx+0x18]`), `0x112DE3A`, `0x112DE22` (`[r12+rsi+0x9F6]`), `imul …,0xB0` |
| 3.3 | It counts **other** entries already on the target team into `ebp` | `0x112DE3F`–`0x112DE76` |
| 3.4 | Capacity per team: `2 → GetMercSlotCount()` (`0x112421C`), `3 → GetMonsterSlotCount()` (`0x1124240`), `1 → GetObserverSlotCount()` (`0x112426C`), anything else → reject | `0x112DE82`–`0x112DEB1` |
| 3.5 | Join is refused when `count >= capacity` unless `bForce` | `0x112DEB1` (`cmp ebp,eax` / `jge 0x112DF6B`) |
| 3.6 | On success logs `EVOLVETWOK-10762 - Setting teamID: %i` and stores the team id | `0x112DEB9`, `0x112DEC7` |
| 3.7 | **Monster capacity getter returns only 0 or 1**: `0` if `[this+0xC798]` bit4 set or `bits0-1 == 0`; otherwise `1` | `0x1124240`–`0x1124268` |
| 3.8 | All 10 identified static call sites pass `bForce = false`; indirect callers were not excluded | call sites `0x111BF06`, `0x112B220`, `0x113E8E6`, `0x113ED88`, `0x17A02E1`, `0x17A0316`, `0x184236E`, `0x184237C`, `0x1842A9C`, `0x184B3E2` |

Team ids confirmed: 1 = observer, 2 = merc, 3 = monster (`Game.log:308` "Setting teamID: 2" for the
human hunter; `root-Game.log:373` "Setting teamID: 3" for the human monster; `team_monster` id `3` at
`0x42CF217`–`0x42CF21C`, `team_merc` id `2`).

**Conclusion:** a second *player* on team 3 is rejected by this check regardless of how many monster
slots exist, because the capacity is a boolean 1.

---

## 4. Runtime slot/team/actor observations (1-player Hunt only)

| # | Observation | Evidence |
|---|-------------|----------|
| 4.1 | Exactly 6 slot entities, created right after `Server.TeamSelection:OnBeginState`, before the roster dump | `Game.log:2099`–`2104` vs `2118`–`2152` |
| 4.2 | Slots are pre-populated with default class/character/perks before any user is attached | `Game.log:2120`–`2162` (merc_00 AssaultD-Hyde … monster_00 RockyBobcicle, stage 1) |
| 4.3 | User occupancy is a slot field, not the team: `user: user_Corvo (userSlot: PlayerSlot_merc_01)` | `Game.log:2135` |
| 4.4 | Human-as-monster occupies `PlayerSlot_monster_00` | `root-Game.log:2232`–`2241` |
| 4.5 | Monster team always has exactly one member, including when a human is the monster | `MarkInitialMonster: monster team has 1 members` at `Game.log:2344`, `2608`, `9481`, `9701`; `root-Game.log:2417`, `2564`; 14.8 MB backup log: all occurrences = 1 |
| 4.6 | `MarkInitialMonster` is generic over N (loops `i < game->vtable[0x308](3)` and calls `vtable[0x310](3,i)`), so it is not itself limited to one — but its count source is always 1 | `0x1A726E4`; count log string `0x24B9908`, `"no monster"` fallback `0x24B9938` |
| 4.7 | Every Hunt layer has exactly one monster spawn (`SpawnPoint_Hunt_Monster_01` / `_Monster_1` / `_Monster_Hunt`); `Level_murder_pits` (non-Hunt) has `SpawnPoint_Monster_01..06` | AssetLibrary `Levels/*/ExternalLayers/Campaign/Hunt.xml`, `Level_murder_pits` |
| 4.8 | Human and AI monsters are the same actor class (`…Bot[AIMonster]`); human control only skips `OnRegisterAgent`/`OnEnableAgent` and adds a third-person camera | `root-Game.log:2365`–`2391` vs AI path `CAgent::DoRevive()` → `OnRegisterAgent()` → `OnEnableAgent()` → `AllocateObject` |
| 4.9 | Round ends on team kill, i.e. rules are keyed to team membership, not player count | `winner:merc reason:ROUND_END_REASON_KILLED_TEAM` (02:22:17) |

---

## 5. Negative evidence (explicit)

1. No `PlayerSlot_monster_01` was created in inspected sessions; the observed
   initialization path fixes `desired[3] = 1`. This is not a whole-program proof.
2. The `"Desired Slot Counts - merc/monster/observer"` log (string `0x240EF10`) appears **nowhere** in
   `Game.log`, `LogBackups/Game.log` or `root-Game.log`; the counts above are from disassembly plus a live
   settings read, not from logs.
3. Unlike the 2012 Lua (`mp_max_monsters` / `mp_max_mercs`, `UpdateRoundRobinTeam` with `maxTeamSize`,
   `TrsMultiplayerTeamUtils.lua:255-296, 357-407, 410-557`), Stage 2 has **no configurable team size**:
   the Lua only keeps `AllowMonsterPlayers` / `campaign_playable_monsters`, and `EntityScheduler.xml`
   only declares `<Class name="PlayerSlot" …/>` and `<Class name="TRSTeam" …/>`.
4. Repo contains no slot allocation code; the only slot handling is log parsing
   (`crates/evolve-desktop/src/runtime_match_perks.rs:39`, `slot()` matching `PlayerSlot_merc_*` /
   `PlayerSlot_monster_*`), so any second-monster support must come from native behaviour, not the repo.
5. No `bForce` team assignment was found among the ten inspected static call sites;
   indirect invocation has not been ruled out.

---

## 6. Ranked next safe read-only probes

1. **In-match settings read** — repeat the `[0x351FCF0]→+0x1A0→+0x38→+0xC798` and `[0x351FD08]+0x440`
   read *during* a live match (not at the menu) and log both values next to the observed slot set.
2. **Entity storage dump mid-match** — `[[0x28D58E8]+0xA8]` length/ids and the game's slot list
   (`+0x10/+0x18`) to show spare entity capacity and that only 6 `PlayerSlot_*` names exist.
3. **Slot component map** — follow only within `0..0x130` of `PlayerSlot_monster_00` to document
   team/class/character/perks/user field offsets, matching the log roster.
4. **Team-full flag consumers** — identify what the two calls at `0x15CF655`/`0x15CF65F`
   (`0x15D6E8`, `0x15D8C0`) mark, since they are the only code that reacts to `desired[teamId]`.
5. **Win/lose audit** — locate consumers of `PlayerMonsterLoseWhenDead` (`0x23DE4F0`, `0x23E4FE0`) and
   `ArenaMode: …` (`0x23DEEF8`) to confirm they assume at most one monster.
6. **Human vs AI monster init diff** — already partially captured (`root-Game.log:2365-2391`); extend to
   the `AllocateObject`/network-ownership lines for both.

---

## 7. Go / No-Go gate for any later reversible experiment

**NO-GO for the observed unmodified offline Hunt path today.** The slot count and
team-admission code are native; the separate spawn constraint is in level assets:

* **(a) Slot supply** — the inspected initialization path uses the literal `1`
  at `0x15CF827` (defaults table) and creates slots when the list is empty. No
  configuration override was identified; entity storage itself is dynamic and
  already held ≥32 objects in a live match, so storage is not the obstacle.
* **(b) Team admission** — `SetTeam` (`0x112DD30`) caps team 3 at `GetMonsterSlotCount()`, which only
  returns 0 or 1 on the inspected path (`0x1124240`) and rejects when
  `existing >= cap` (`0x112DEB1`). Inspected static callers pass `bForce = false`.
* **(c) Spawn supply** — inspected Hunt layers have one monster spawn entity;
  a level-asset edit could provide another, but cannot by itself change (a) or (b).

Additionally unverified and required for a real second monster *player*: per-actor monster init
(camera/HUD/`AIMonster` bot wiring), monster singleton assumptions in rules (`PlayerMonsterLoseWhenDead`),
and arena/kill-based win counters keyed to a single monster.

**GO conditions for a two-player prototype** (all must be demonstrated): an extra
monster player slot, team admission for both players (or a validated alternate
opponent-team model), and a valid spawn policy. None is established in this build.

**One discriminating experiment, only if separately authorized and designed with
restoration and build guards** (not done here): changing the slot-count literal from
`1` to `2` in a disposable offline session, then checking
`Game.log` for a 7th slot `PlayerSlot_monster_01` at the `AllocateObject` block.
If it appears, that would test slot creation independently of team admission;
it would **not** prove a second player can join team 3. No outcome is guaranteed
without running the experiment.

---

### Caveat
All gameplay observations are from **one local player, offline Hunt**. Nothing here generalises to
multiplayer/matchmaking. The observed "monster team has 1 members" is consistent
with both the one-slot initialization and 0/1 admission cap, not an independent
proof of what a different lobby could support.
