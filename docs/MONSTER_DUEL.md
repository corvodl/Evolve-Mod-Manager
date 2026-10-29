# Monster vs Monster: offline implementation investigation

Status (2026-10-29): **research / read-only probe**, not a playable mode or an
entity-spawning implementation. Nothing in this branch calls CryEngine functions
or modifies the game process. Do not infer Stage 2 support from older
Evolve/Legacy modes.

## Updated constraint: Stage 2 has no Defend / Second Monster effect

The user confirmed these historical modes/effects are not available in the
current Stage 2 client. Do not use their presence as the first live acceptance
test, and do not claim the Stage 2 runtime still has their minion spawning path.
The initial probe now operates in an **ordinary offline Stage 2 match with
hunter bots**, validating multi-actor discovery and damage-field changes.

## Historical lead (requires asset/runtime verification)

Older Evolve modes already use additional monster-like AI actors:
- Defend periodically spawns Stage 1 Goliath minions alongside the primary
  monster (typically max two).
- The **Second Monster** map effect places a monster minion with the primary
  monster in certain custom matches.

These are game-managed **minions**, not evidence that a fully initialized,
player-controlled second monster is supported. First determine whether
the current Stage 2 client exposes those modes/effects or retains their
data/runtime implementation.

Historical references:
- https://evolve.fandom.com/wiki/Defend_Mode
- https://primagames.com/eguides/evolve-official-strategy-guide/map-effects/favors-monsters
- https://primagames.com/eguides/evolve-official-strategy-guide/basics/map-effects

CryEngine engine-level API references (NOT confirmed Evolve ABI):
- https://www.cryengine.com/docs/static/engines/cryengine-3/categories/17399809/pages/17074769
  (IEntitySystem::SpawnEntity)
- https://subscription.packtpub.com/book/game-development/9781849695909/5/ch05lvl1sec35/introducing-the-actor-system
  (IActorSystem::CreateActor, which also requires game-object/network ownership)

## What is already available in this repository

- `crates/evolve-desktop/src/runtime_actors.rs` resolves the local
  `Game -> Framework -> Client -> Actor` chain and its IEntity; derives an
  entity-to-actor backlink from this verified local pair and uses it to
  enumerate nonlocal actor candidates; resolves *entity instance names* using
  `[entity+0x124]` and the build-specific name table.
- It recognizes stage at actor+0x1260 when `0x0101/0202/0303`; health
  at +0x198/+0x1A0; armor at +0x1038/+0x103C after plausibility checks.
  These are candidate field offsets from an offline tested build, **not**
  universally correct for all monster variants/minions.
- `crates/evolve-desktop/src/runtime_experiments.rs` has Spawn/Swap
  *previews only*. It explicitly does not know the Evolve class registry,
  initialization functions, ownership or authority interfaces.

## Reproducible read-only CE probe

Open Evolve Stage 2 offline, then in Cheat Engine's Lua Engine execute:

`scripts/experiments/monster_duel_preflight.lua`

The script automatically attaches to Evolve.exe, resolves local actor/entity,
discovers an entity-to-actor backlink using the same approach as the manager,
enumerates the bounded 16-bit-index entity table, and collects up to three
user-prompted snapshots with actor IDs, entity names, linked actor addresses,
positions, and plausible stage/health/armor fields. It never writes game memory
or invokes engine functions.

1. Baseline: local playable monster in a normal **offline Stage 2 match with
   hunter bots**. This does not require Defend or any legacy map effect.
2. Approach a visible hunter bot, do not attack, and capture a second snapshot
   **in the same match**.
3. Hit that same hunter bot with one monster ability or melee attack; capture
   a third snapshot quickly, without changing matches.

This provides a control case for Evolve's actual Stage 2 actor registry,
health fields and monster-to-hunter damage path. It is **not** evidence that
a second monster can be created or damaged.

Provide complete Lua output. A delta in health/armor is evidence only when the
same live actor pointer and entity ID are still present; it doesn't by itself
prove who caused the damage. A zero delta does not prove the attack was a valid
hit or that faction filtering rejected it.

Since historical effects are absent, inspect Stage 2 PAKs and, separately,
Legacy PAKs supplied by the user for monster actor class/archetype definitions,
registered entity classes and native spawn initialization. The manager already
classifies gameplay paths including `Libs/GameRules/`,
`Libs/EntityArchetypes/`, `Libs/AI/`, `Libs/Actor/`, and `Entities/`.
These are **search scopes**, not confirmed locations of spawn code.

Compare any Legacy Defend/Second Monster definitions to Stage 2 before attempting
a port. Data-only restoration will not work if the underlying native game-mode
logic and runtime classes were removed. No guessing at ABI vtable indices.

## Actual implementation milestones and acceptance tests

1. **Establish current Stage 2 baseline.** In normal offline matches, identify
   local monster and AI hunter actors, compare entity names, fields, actor
   backlinks, and a controlled monster-to-hunter damage event.
2. **Research monster class/spawn lifecycle.** Compare Stage 2 monster
   class/archetype definitions with any available Legacy Defend/Second Monster
   assets. Determine the actor registration/creation, AI controller, lifetime,
   authority and cleanup interfaces. Do not assume a PAK-only approach.
3. **Make two monsters coexist offline.** Validate distinct IDs/entities,
   physics, health/armor, animations, behavior, and clean despawn. Do not
   create a raw memory clone or overwrite an existing player's actor pointer.
4. **Investigate targeting and damage.** Capture controlled attacks and health
   changes, infer whether game rules/team/faction filters prevent friendly
   monster damage, and locate the actual decision point before changing it.
5. **Combat prototype.** Support Goliath vs Goliath with intentional opposing
   allegiance and damage, reliable ability collision/knockback, and a repeatable
   reset mechanism. Only add a functional test button after calls are verified.
6. **Player-vs-player prototype.** Separate player/channel ownership,
   replication, input, spawn lifecycle, spectators, and reconnect handling.
   This is more difficult than an offline monster-vs-AI duel.
7. **Mode rules.** Remove 4v1 assumptions from teams, victory, round start,
   ability unlocks/evolution, HUD and score.

### Decision gate

Do not label this a working Monster vs Monster mode unless two fully initialized
monsters can remain alive in one offline match and a deliberate attack from one
changes the other's verified health/armor (followed by successful cleanup).

### Known limitations of the first probe

- Entity names are **instance names**, not class IDs.
- Nonlocal actors appear only if the same verified local backlink applies.
- Normalized-looking health/stage data in a minion is **not** sufficient
  to prove complete actor initialization.
- Scan is bounded (8192 live entities, 256 stored actors, 70 displayed).
- The script requires compatible current Stage 2 offsets and an attached
  offline Evolve.exe. It has not been tested against an in-game session.
- It observes monster-versus-hunter combat only as a control case; does
  not infer monster-versus-monster damage filtering from that case.
- If the normal match contains no enumerated nonlocal hunter actors, investigate
  the entity-to-actor backlink or name registry rather than inferring that
  the hunter bots are not present.
