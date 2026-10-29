# Monster 1v1 Arena — isolated test

Status: NOT a playable 1v1; test instrumentation and reversible XML configuration only.

The target is two independently controlled monsters on opposing sides of a 1v1 Arena, using the existing Arena dome and best-of-round mechanics. The user permits replacing normal Arena temporarily; the final implementation must be an additional mode.

Native constraints: Evolve Stage 2 normally creates four merc slots and one monster slot. Patching the native monster desired-slot count to two has repeatedly crashed during level loading before PlayerSlot allocation is logged. Reusing PlayerSlot_merc_00 for a second monster is a hypothesis, not a demonstrated implementation. Actor creation, network ownership, damage, round victory, and independent input remain unverified.

## Test 1 — Arena XML override

Run: python scripts/experiments/build_arena_override.py --libs-zip libs.zip --output scratch-arena

The guarded builder creates a single libs/GameRules/ArenaMode.xml override changing required wins from 2 to 1, battle time 6 to 5 minutes, start hold 5 to 3 seconds, and no-fire time 3 to 2 seconds. This is an observable smoke test, NOT a monster-versus-monster rules implementation. Custom-game options can override these values.

Use the manager's isolated, authenticated signed-PAK project workflow targeting the existing libs.pak; do not overwrite original game PAKs with a raw ZIP or stage unsigned assets. Test stock Arena on Ice Alpine offline, then disable the test profile and verify recovery.

## Test 2 — read-only CE instrumentation

Run scripts/experiments/arena_slot_probe.lua in Cheat Engine Lua Engine while attached to an OFFLINE Arena match. It monitors the entity table and name resolver for PlayerSlot_merc_00..03, PlayerSlot_monster_00/01, spectator slot, and team objects. Call arenaSlotSnapshot('in-arena') after spawn and repeat after state transitions. Log: %TEMP%\EvolveArenaSlotProbe.log. It does not write memory, call into game code, inject hooks, or run the debugger. The scanner has bounded memory accesses and requires Evolve.exe build 3.6.1.2538.

## Gate for real Monster vs Monster

Identify and validate slot character/class initialization, controller ownership and replication. Test that a monster actor can genuinely occupy a merc slot (not just have its display name changed), survive, use abilities, and take damage from the monster on the native monster slot. Verify separate users and win conditions, plus reliable teardown. Do not claim success until demonstrated in-game.

Once verified, add a distinct Monster 1v1 mode instead of replacing normal Arena.

## Corrected target: EXACTLY TWO HUMAN PARTICIPANTS

**No 4v1 logic or hunter bots are allowed in the Monster 1v1 Arena ruleset.** Temporary validation may override existing Arena; the finished game mode must be separately selectable.

| Internal slot | Behavior in Monster 1v1 |
| --- | --- |
| PlayerSlot_monster_00 | Human Player A controlling a monster (native monster team) |
| PlayerSlot_merc_00 | Human Player B controlling a monster (opposing team; experimental repurposing) |
| PlayerSlot_merc_01 | Disabled: no selection, bot, user assignment or player actor |
| PlayerSlot_merc_02 | Disabled: no selection, bot, user assignment or player actor |
| PlayerSlot_merc_03 | Disabled: no selection, bot, user assignment or player actor |
| spectator_00 | May exist as an internal object, but must not permit a third participant |

**Allocated is not the same as active.** The engine may require the unused merc slot objects to exist. The requirement is to make them unavailable and keep them unoccupied; forcing their native allocation count to zero without mapping dependencies would be unsafe.

Native game-mode implementation requirements:
1. Lobby/matchmaking capacity is exactly 2 humans. It must neither start with one player nor accept a third (including late join/direct join). No automatic hunter bot fill, no spectator human joining as a third participant. Test with three clients.
2. Map Player A to monster_00 and Player B to merc_00; reject every other playable class/slot for this mode.
3. Assign a full monster actor to each user, with separate ownership, replication and input. Keep native monster slot allocation at 1 pending the merc-slot experiment; never patch it to 2 as an interim workaround.
4. Make their allegiances opposing despite both being monsters. Verify melee, abilities, knockback, armor/health, death and win/loss from both perspectives.
5. Disable hunter role selection, hunter AI spawning, reinforcements and 4v1-dependent round endings for this mode only.
6. On leaving experimental Arena, revert all allocations, role eligibility, game rules and matchmaking to normal. No changes to Hunt, custom 4v1 or existing public matchmaking behavior.

### Automated read-only log gate

Run \`python scripts/experiments/check_arena_duel_log.py "<Game.log>"\` on a 1v1 candidate match. It flags one/three human users, any hunter character in disabled merc_01..03, missing owner on either active slot, wrong actor class/team, or an unexpected monster_01 slot. The supplied normal Hunt control log correctly **fails** this check. Run unit tests with \`python -m unittest discover -s tests -p 'test_check_arena_duel_log.py'\`.

A passing log is **only preliminary**: it cannot demonstrate the matchmaking maximum, bot absence, ownership/replication, damage or third-client rejection. Those require a real two-client test plus an attempted third connection. At present this is a **requirements/test update**, not executable two-human matchmaking enforcement or monster-on-merc initialization.
