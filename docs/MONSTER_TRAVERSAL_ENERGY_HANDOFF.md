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

Read-only capture artifacts (temporary local evidence, **not** portable project fixtures):

- `C:\Users\billn\AppData\Local\Temp\opencode\wraith-traversal-20261029-controlled\`
- `C:\Users\billn\AppData\Local\Temp\opencode\goliath-traversal-20261029-1\`
- `C:\Users\billn\AppData\Local\Temp\opencode\behemoth-traversal-20261029-controlled\`
- `C:\Users\billn\AppData\Local\Temp\opencode\behemoth-traversal-20261029-stage2\`

Each directory contains `meta.json` frame metadata and `actor.bin` /
`movement.bin` snapshots. These samples identify decoded energy behavior, but
**do not** identify a recharge-rate address, prove a distinct authoritative
charge count, or validate any control over recharge speed. The native helper
above identifies named recharge attributes and a calculation, not a verified
external implementation address. Further recharge-storage tracing remains
separate research; no runtime code changes are proposed by this handoff.
