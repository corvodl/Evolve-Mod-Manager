# Source-backed Level Editor effects: preview vs. native parity

## Scope

This work starts after merged PR #156. The CryEngine 3.5.8 FreeSDK is a
**general historical reference**, not proof of Evolve's exact game renderer
or a public, redistributable dependency. No proprietary SDK/game binaries,
shaders, map files, character assets, credentials or converted game data are
committed or bundled.

## Implemented: bounded distance-only fog preview

1. Resolve `LevelData/Environment.xml` (or the map's sibling `Environment.xml`)
   using the exact selected map virtual folder and existing unique-build
   ownership checks; a map-relative `level.pak` fallback is permitted only
   when the source ownership checks already permit it. No global filename
   search or mixing across game builds.
2. Read only bounded (4 MiB) regular files. Parse plain XML and CryXmlB
   through the existing guarded decoder; accept one valid `Fog` definition
   with finite, bounded `ViewDistance` and `LDRGlobalDensMult`. Missing,
   oversized, linked, malformed or ambiguous sources do not enable preview.
3. Require the same world's verified `TimeOfDay.xml` with an authored
   `Fog color` variable. Color follows the existing manual 0–24h slider.
   If either original source is absent or density is zero, the GPU uniform
   is zero and fog is disabled.
4. Apply a **non-native bounded exponential distance heuristic**, not an
   asserted CryEngine law, *only* in `world_instances.wgsl` after material
   shading. The world camera/fragment positions determine distance.
   Preserve opaque depth/alpha, foliage alpha-test and transparent material
   source-opacity behavior. The separate Model Viewer shader is untouched.
5. Label the feature **Distance-only fog PREVIEW** in Scene Settings and
   diagnostics. The original XMLs and authored PAK members remain read-only.

The heuristic is `factor = clamp(1 - exp(-(distance / ViewDistance) *
(0.35 * LDRGlobalDensMult)), 0, 0.92)`, blended toward the TimeOfDay fog
color. Its 0.35 factor and 0.92 bound are deliberately chosen preview
controls; they have **not** been recovered from Evolve shaders or validated
as the game's physical fog model.

Known privately held example *schema values* (not copied into code):
Ice_Crashsite 4500 / 3, Forest_Ruins 20000 / 3, Lava_Rivers 2400 / 1.
These establish useful parser input cases, **not** the native fog formula.

## Acceptance gates before claiming native fidelity

| System | Implemented behavior | Required for native-parity claim |
| --- | --- | --- |
| Emissive | Original emissive MTL maps and bounded source strengths preview | In-game comparison of material response, light spill and native bloom/HDR |
| Particles | Read-only effect/placement inventory | Decode effect nodes, GPU/CPU emitters, spawning, attachment, LOD, sorting, source textures and in-game reference sequences |
| Time of day | Source-backed sun/sky curves with approximate tangents; original fog color | Verified Evolve curve interpolation, light direction, atmosphere, exposure and authored timeline control |
| Fog | This distance-only preview | Native height/volumetric fog and scattering across indoor/outdoor maps |
| Lights/shadows | Simple viewport sun and ambient lighting | Original authored local lights, dynamic shadows, reflection and occlusion behavior |
| HDR/GI | Not implemented | Source-equivalent exposure, tone mapping, light transport/GI and validated rendering |

**Visual acceptance:** inspect original Stage 2 Evolve maps at matched camera
position/time-of-day and capture comparisons for Ice_Crashsite, Forest_Ruins,
Lava_Rivers, plus low-light emissive and particle-heavy scenes. Compare with
native game rendering, inspect any discrepancies, and only promote
source-verified behavior to engine-parity after verification.

**Developer checks:** formatting, desktop compilation and tests, core tests,
parser bounds, malformed source handling, shader WGSL/Naga parse, zero-uniform
fallback and time-of-day changes. Tests use synthetic XML only. Real owned game
sources are permitted for local read-only schema checks, not distribution.
