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