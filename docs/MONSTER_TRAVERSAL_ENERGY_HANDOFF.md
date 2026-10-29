# Monster traversal energy: read-only Stage 2 handoff

**Status:** observed and decoded in read-only captures; no traversal-energy
edit, freeze, or recharge control has been implemented or validated. This is a
different resource from the evolution energy documented in
[`MONSTER_EVOLUTION_ENERGY_HANDOFF.md`](MONSTER_EVOLUTION_ENERGY_HANDOFF.md).
Do not substitute evolution offsets or encoding for traversal offsets.

## Verified field identification

Read-only native disassembly at `Evolve.exe` RVA `0x16645A6` confirms the
traversal-field encoding. For the validated local **actor** in the inspected
build, read the 32-bit encoded values and the process-specific key:

| Value | Encoded storage | Read-only decode |
| --- | --- | --- |
| Current traversal energy | `actor+0x1754`, UInt32 | `f32::from_bits(raw ^ key ^ 0x0093F6E4)` |
| Maximum traversal energy | `actor+0x1758`, UInt32 | `f32::from_bits(rotate_left(raw ^ key, 7))` |

Here `key` is read from `Evolve.exe+0x28D5AE0`; it is **not** a reusable
constant or an address to persist across sessions. Decoded maximum was `100`
in the observed traversal captures. The current and maximum fields use
different transforms. Field positions are actor-relative, not entity- or
movement-relative. The disassembly establishes the decode for this build,
**not** the safety or effectiveness of an external write. No write or native
setter was tested. Revalidate build, process, actor identity, and key in any
future investigation.

## Capture observations

The following are approximate changes in decoded current energy over timed,
read-only traces, not configured gameplay constants or verified recharge
parameters:

| Monster / capture stage | Observed use and recovery |
| --- | --- |
| Wraith (`TeleporterABot`), Stage 2 | Three-dash bursts; approximately `30` energy spent per dash and `7.7` energy/s recharge in the observed conditions. Recharge-perk state is unknown. This trace alone does not establish a separate charge-count field. |
| Goliath, Stages 1 and 2 | Approximately `30` energy per traversal jump and `6.25` energy/s recharge, with no recharge perk in the observed conditions. |
| Behemoth (`RockyABot`), Stages 1 and 2 | Roll starts with approximately `10` energy upfront, then drains approximately `6` energy/s while active; approximately `5` energy/s recharge when recovering. This is a continuous roll drain, not evidence of discrete dash charges. |

The separate Stage 2 Behemoth capture contains **647 frames, all with stage
word `0x202`** (about 64.9 seconds of samples). The Stage 1 Behemoth capture
has stage word `0x101`; the Goliath capture includes both `0x101` and `0x202`.
Rates above reflect the captured gameplay conditions and should not be
treated as fixed across perks, rules, monsters, stages, or builds.

## Recharge calculation: native lead, not a writable offset

Read-only disassembly of the native recharge helper at RVA `0x1661F88`,
called during the actor update near RVA `0x1664632`, shows that the game
**computes** a recharge amount each update rather than simply reading a
confirmed `actor+offset` recharge-rate float. The helper decodes the maximum
at `actor+0x1758`, retrieves a modifier from a runtime component (query ID
`0x19`), adds `1.0`, and obtains an actor attribute by name through the
getter at RVA `0x1A4A9F8`. Depending on a runtime condition, that attribute
is `specialEnergyRechargeSecs` or `specialEnergyRechargeSecsCombat`. It
divides the decoded maximum by the chosen seconds value, multiplies by the
modifier-plus-one, and can additionally apply a conditional `0.5` multiplier.
The update then multiplies the resulting rate by elapsed time and adds it to
the current resource, bounded by the decoded maximum.

This explains why measured rates need not be identical across monsters or
conditions. The non-combat observations are *consistent* with roughly 16 s
(Goliath), 13 s (Wraith), and 20 s (Behemoth) for a 0-to-100 refill with no
other modifiers, but those are **inferred effective times**, not validated
attribute values. The attribute's backing storage, modifier meaning, combat
branch condition, and a safe way to read or change an effective recharge rate
externally have **not** been established. In particular, the nearby
`actor+0x175C` is not a verified recharge-rate field. Do not present any
recharge setting or editable address on the strength of this trace alone.

### Three-perk Behemoth comparison

In a separate 1,792-frame read-only Behemoth fight capture, the player reported
three simultaneously equipped traversal-recharge bonuses of **6%, 17%, and
10%**. The Stage 2 marker was `0x55FA0202` for 948 frames and the Stage 3
marker was `0x55FA0303` for 844 frames; compare the low 16 bits, not the
entire stage word. Median decoded recharge slopes were approximately
`6.66` energy/s at Stage 2 and `6.65` energy/s at Stage 3, versus `5.0`
energy/s in the earlier no-perk Behemoth captures. Continuous roll drain
remained approximately `6.0` energy/s. This is consistent with three
additive `MonsterSpecialRechargeModifier` bonuses:
`5.0 * (1 + 0.06 + 0.17 + 0.10) = 6.65` energy/s. It is not evidence for a
separate editable recharge-rate field. The installed perk effect sets
`MPerkBrawlerLowEffect.effectset`, `MPerkStaminaRegenMediumEffect.effectset`,
and `MPerkBrawlerHighEffect.effectset` have corresponding tier-3 bonuses
`0.06`, `0.17`, and `0.10`; individual equipped item identities were not
independently verified from the runtime capture.

## Generic damage-reduction effect (separate research)

The installed `libs/Actor/Monsters/RockyA.actorparams` defines Behemoth's
`ReactiveEffect` with `fDamageThreshold=65`, `nStackLimit=15`, and
`sEffectSet=RockyReactiveArmorEffect`. The corresponding
`libs/EffectSets/RockyReactiveArmorEffect.effectset` applies the generic
`ArmorModifier` game property with value `0.03`, duration `6`, and self
stacking. Fifteen 3% stacks would sum to 45%, but the actor capture does
**not** establish the exact runtime damage or stacking arithmetic.

This property is not Behemoth-exclusive: monster perk effect sets such as
`MPerkBrawlerHighEffect.effectset` (tier-3 `ArmorModifier=0.08`) and
`MPerkDamageReductionHighEffect.effectset` (`0.12`) apply it to the owner.
Thus ordinary monster perk selection can supply damage reduction to other
monsters. The fight capture's first `0x2400` actor bytes did not reveal a
validated `0..15` passive-stack count or a float rising in 3% increments.
The effective property is represented in a separate actor-linked property
component (below); Behemoth's passive stack count was **not** captured from
that component. No game-thread effect-application API was verified. Do
**not** copy Behemoth's reactive configuration, edit a guessed actor stat,
substitute an item ID, or claim arbitrary runtime activation on another
monster from these observations. No activation or write was attempted.

### Read-only non-Behemoth armor baseline

A second, identity-guarded capture of local Meteor Goliath
(`GoliathMeteorBot`) followed the property component pointer at
`actor+0x1130` (vtable RVA `0x24A5020` in this build). The user reported
equipped armor modifiers of 4% (Brawler), 9% and 12%. Native property-name
registration maps `MonsterSpecialRechargeModifier` to ID `0x19` and
`ArmorModifier` to ID `0x49`. Read-only disassembly of property getter RVA
`0x19DB5E0` and aggregator RVA `0x19DB7C8` identifies:

- `component+0x910`: number of active property entries. The entries begin at
  `component+0x50`, each `0x1C` bytes. The first UInt32 is the property ID;
  the entry's encoded Float32 at `entry+0xC` decodes as
  `f32::from_bits(rotate_left(raw ^ key, 4))` for this process/build.
- The computed-value cache for property ID `p` has a validity byte at
  `component+0x919+16*p` and an encoded Float32 at
  `component+0x91C+16*p`. **Only if valid**, decode the value as
  `f32::from_bits(raw ^ key ^ 0x00AD3091)`.

All 896 frames in the guarded Meteor Goliath property capture had valid
`ArmorModifier` (`p=0x49`) cache value `0.25` and three corresponding
entries `0.04`, `0.09`, `0.12`. Separately, `p=0x19` cached `0.06` and had
one `0.06` entry. These observations distinguish equipped-source entries
from the computed aggregate and demonstrate the generic armor property on a
**non-Behemoth** monster. A cache is **not** an authoritative writable
source: the game can recompute it from effect entries. This establishes
neither a safe effect-add call nor damage-mitigation behavior under a
controlled identical hit. Validate actor, component identity, build and
process-specific key before any read; do not reuse these offsets for writes.

### Runtime application path: research only

Read-only disassembly finds an internal property-entry upsert at RVA
`0x19DAB90`: it finds an empty/matching entry, encodes its value, updates
entry metadata/count, invalidates the computed property's cache, and notifies
the owner. An effect-lifecycle path near RVA `0x1AC7889` reaches a related
entry writer (`0x19DB090`) and removes its entry on effect teardown through
the component's removal path. This is further evidence that **applying and
later removing a real effect** is preferable to overwriting either the
cached aggregate or a single entry's raw bytes. Neither routine has a
validated external ABI, calling thread, ownership token, timer/stacking
semantics, or cross-monster application test. Do not call it or synthesize
entry writes on the basis of disassembly alone. Normal, game-supported
monster-perk selection is the currently demonstrated way to grant generic
damage reduction to another monster; arbitrary extra runtime application
remains open research.

Read-only capture artifacts (temporary local evidence, **not** portable project fixtures):

- `C:\Users\billn\AppData\Local\Temp\opencode\wraith-traversal-20261029-controlled\`
- `C:\Users\billn\AppData\Local\Temp\opencode\goliath-traversal-20261029-1\`
- `C:\Users\billn\AppData\Local\Temp\opencode\behemoth-traversal-20261029-controlled\`
- `C:\Users\billn\AppData\Local\Temp\opencode\behemoth-traversal-20261029-stage2\`
- `C:\Users\billn\AppData\Local\Temp\opencode\behemoth-passive-20261029-fight\`
- `C:\Users\billn\AppData\Local\Temp\opencode\meteor-goliath-armor-20261029-baseline\`
- `C:\Users\billn\AppData\Local\Temp\opencode\meteor-goliath-property-cache-20261029-1\`

The traversal/fight directories contain `meta.json` frame metadata and
`actor.bin` / `movement.bin` snapshots; the Meteor Goliath property capture
contains `meta.json` and `component.bin`. These samples identify decoded
energy and property behavior, but
**do not** identify a recharge-rate address, prove a distinct authoritative
charge count, or validate any control over recharge speed. The native helper
above identifies named recharge attributes and a calculation, not a verified
external implementation address. Further recharge-storage tracing remains
separate research; no runtime code changes are proposed by this handoff.
