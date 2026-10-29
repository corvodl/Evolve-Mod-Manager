# Stage 2 monster evolution energy: runtime and Developer-window handoff

**Status:** read-only field identification, **not** a validated write feature.
This handoff is for an offline-only manager Developer control. No energy field was
written, frozen, or changed by a native call in this research. All addresses and
RVA-based layouts require build validation; absolute addresses, actor IDs, and
the encoding key must be resolved anew after every process or world change.
See `docs/RUNTIME_MEMORY_MANAGER_HANDOFF.md` for the broader runtime safety plan.

## Verified observations

The live Stage 2 script bindings named `GetMonsterEnergy` and
`GetMonsterEnergyMax` are registered at string RVAs `0x24DC7A8` and `0x24DC790`.
Their wrappers call native getters at RVAs `0x1661C18` and `0x1661C34`,
respectively. Read-only disassembly of the decrypted, running image shows the
getters decode two 32-bit fields of the player actor. The shared key is read at
`Evolve.exe+0x28D5AE0`, **not** embedded in the manager as a constant:

| Value | Encoded storage | Native getter / decode |
| --- | --- | --- |
| Current energy | `actor+0x16FC`, UInt32 | `f32::from_bits(rotate_right(raw ^ key, 9))` |
| Maximum energy | `actor+0x1700`, UInt32 | `f32::from_bits(rotate_left(raw ^ key, 16))` |

The inverse for a proposed current-energy value `x` is
`raw = rotate_left(x.to_bits(), 9) ^ key`. **Do not** write a plain Float32 to
`+0x16FC`. The maximum is read-only in the proposed control; its different
encoding would require a separate, unvalidated change. The field locations are
relative to the independently validated local player **actor**, not the entity,
physics object, inventory, or player slot.

| Read-only session | Decoded result | Independent observations |
| --- | --- | --- |
| Goliath, first capture | Stage 1 energy `14/24` to `24/24`; stage 2 `0/28` then `10/28`. | `actor+0x484` rose to `180` before evolution and reset, but is not the same value. |
| Goliath, second capture | Stage 1 `17/24` to `24/24`; stage 2 `0/28` to `28/28`. | `+0x484` reached only `170` at stage-1 full and `290` at stage-2 full: it cannot be used as a fixed bar threshold. |
| Wraith (`TeleporterABot`) | Stage 1 `0/24` to `21/24`, rising during feeding. | The verifier stopped on process/world unavailability; this is not a verified Wraith stage-2 result. |
| Elder Kraken (`KrakenElderBot`), **new process** | Stage 1 `6/24` to `24/24`, then stage 2 `0/28` to `11/28`. | Key was **different** (`0xC0B73BB2` vs `0xB805B8B7` in the earlier process). Stage field read `0xFA360101` / `0xFA360202`, with stage encoding in the low 16 bits. The 180-second read-only probe completed without a crash. |
| Stage 3 Goliath | Getters decode `0/0`. | The evolution bar disappears at the final stage; the separate `+0x484` count can continue changing. |

The decoded stage-1/stage-2 maxima also agree with the installed
`AssetLibrary/.../libs/GameRules/ScenarioData.xml` monster attributes
`fMaxEnergyStage1="24"` and `fMaxEnergyStage2="28"`. The local captures are
temporary investigation artifacts at
`C:\Users\billn\AppData\Local\Temp\opencode\goliath-evolution-20261029-{1,2}\`;
the read-only Wraith/Elder verification was streamed from
`verify_wraith_energy_readonly.py`. These paths are **not** portable fixtures.
Historical Elder Kraken readout: stage-2 current `21`, max `28`, key
`0xC0B73BB2`, raw current `0x90B73B31`, raw max `0xC0B77A52`; treat every
literal as an example from that process, not a reusable value. The corpse
counters `actor+0x1B00/+0x1B08` mark new-corpse
interactions and persist across stages; they are not energy either. Combat can
also increase evolution energy, so correlating feeding alone is insufficient.

## Proposed Developer-window implementation

1. **Probe only.** Reuse `runtime_memory::inspect` and
   `runtime_actors::inspect` for a fresh PID/base and local actor. Additionally
   verify the running image/build identity, actor vtable RVA `0x24B3790`, entity
   ID (`entity+0x18`), actor/entity backlink (`actor+0x38`), movement backlink
   (`actor+0x240` then `+0x38`), recognized local monster name, and a known
   offline match. Do not treat a password-gated Developer page as proof of an
   offline session. Fail closed if build identity or authority cannot be shown.
2. Read key and raw fields, decode both as above, and display current/max plus
   stage and read-only status. Accept only finite values and plausible ranges;
   stage 1 expects max `24`, stage 2 `28` **for the inspected rules/build**.
   For stage 3 (`max=0`) disable energy edits rather than divide by zero.
   Stage is read from `actor+0x1260`: compare its **low 16 bits** against
   `0x0101/0x0202/0x0303`, but validate the complete actor separately; Elder
   Kraken had changing upper bits. Existing `runtime_actors.rs::stage_number`
   checks the whole word, so it can reject this valid actor and should not be
   reused unchanged for this feature.
3. **Arm a single edit explicitly.** User supplies a finite Float32 within
   `[0, freshly decoded max]`. Record a probe token containing PID, process
   start/build identity, module base, entity ID, actor pointer, stage, key,
   max, original 4-byte current field, and expiry. Neither the address nor the
   encoded value may be saved as a mod setting or applied after a restart.
4. **Revalidate immediately before write:** same actor/entity, stage, key and
   max, offline status, and unchanged original raw bytes. Compute
   `rotate_left(desired.to_bits(), 9) ^ key` only then. The existing
   `runtime_live::write_four` path does read/compare, writes four bytes once,
   and reads back; use it rather than a generic plain-float writer. It is a
   **compare-then-write**, not an atomic CAS against the game thread. A game
   update between compare and write is still possible; report this limitation.
5. **Observe, do not freeze.** Read back immediately and shortly afterward;
   compare decoded value and, if visible, HUD behavior. The game/server may
   clamp, quantize, overwrite, or ignore an external write. Treat any of those
   outcomes as evidence, not a reason to repeatedly write. Never edit max,
   stage, corpse counters, or `+0x484` as part of this control.
6. **Conditional restoration.** Preserve original raw bits for an explicit
   restore attempt, but restore only while the *same* process, actor, stage, key,
   and max remain valid **and** the field still equals the test bytes. If the
   game legitimately advanced energy, do not overwrite its value. Stop and
   clear the token on actor unload, evolution, process exit, or identity drift.

**Acceptance gates before enabling the button:** unit tests for the rotate/XOR
round trips (including two distinct keys), `0/24`, `24/24`, `0/28`, `28/28`,
`0/0`, invalid floats, and upper stage bits; fake-process tests for stale PID,
changed actor/entity/key/stage/max/baseline and conditional restore; an
explicitly authorized, disposable **offline** one-write experiment with HUD
observation and no crash. No such write experiment has been completed yet.

## Next research: Monster Traversal (separate, read-only)

The traversal resource should **not** be inferred from evolution energy or
assumed to have one shared shape. Wraith has three rechargeable dashes;
Goliath has three rechargeable traversal jumps; Behemoth's roll is continuous
and drains a resource while active. In fresh offline matches, independently
identify each local actor, capture a stationary baseline, then record precisely
timed uses, recharge intervals, depletion and recovery. Search actor plus
validated movement/ability components for charges, fractional energy, cooldown
timestamps and roll state; distinguish a UI charge display from authoritative
state. Compare across stages and at least two monsters before proposing common
fields. No writes, freezes, or inferred recharge-rate controls in this phase.
