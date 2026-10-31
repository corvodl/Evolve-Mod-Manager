# Height-aware fog parity research (follow-up to merged PR #157)

This draft PR implements the first **source-backed, analytical height-fog
preview**, not Evolve-native shader equivalence. PR #157 added read-only,
bounded exact-map `Environment.xml` parsing, source TimeOfDay fog color,
and a fallback distance-only fog preview.

## Implemented in this branch

- Sample TimeOfDay **Fog height (bottom/top)** and **Fog density
  (bottom/top)** from the selected map's verified TimeOfDay profile; accept
  explicit label variants only and reject missing, non-finite, reversed or
  out-of-bounds source curves. Source values are not changed.
- Add a separate world-only fog-height GPU uniform. Its enable bit stays
  zero unless a complete original profile exists **and** PR #157's exact
  map Environment and fog-color checks pass.
- Implement an **analytical integral** of an exponentially varying vertical
  density profile along camera-to-fragment world-space rays. Horizontal rays
  use a stable near-zero expansion; all exponentials and optical depths are
  bounded to protect the wgpu preview. Works on opaque surfaces, masked
  foliage and authored-transparent world materials. Model Viewer is
  unaffected.
- Preserve the existing distance-only fog heuristic when native height
  curves are absent. Scene Settings shows which mode is currently active.
- Include synthetic XML and height-ray reference tests and Naga WGSL
  semantic validation. Zero-density height endpoints do not produce
  invented exponential profiles: those profiles use the labelled distance
  fallback. Source fog variable names and their selected-hour values are
  shown read-only in Scene Settings for Evolve-specific schema investigation,
  without promoting unfamiliar names into runtime aliases. No SDK runtime,
  proprietary file or user installation is required.

## What this is *not*

- `Environment.xml` `ViewDistance` is **view distance**, not proven
  native Evolve extinction. `LDRGlobalDensMult` is deprecated in general
  CryEngine documentation. The current uniform still uses them as an
  intentionally labelled **preview calibration**, and the physical fog
  integral supplies only altitude-dependent shape. It is **not** the true
  Evolve extinction, sunlight in-scattering or fog blending equation.
- Evolve Stage 2 engine customizations, the exact TimeOfDay variable names,
  original fog shaders, sun/radial scattering, local fog volumes, fog-specific
  shadows, clip volumes, HDR, tone mapping and volumetric voxel fog
  remain **unverified or unimplemented**.
- The older general CryEngine 3.5.8 FreeSDK interfaces are architectural
  references only; they are not original Evolve runtime binaries and are
  not distributed with the manager. Sources used in repo tests are
  synthetic rather than redistributed original map or SDK data.

## Acceptance work

1. Read-only inspect original Ice_Crashsite, Forest_Ruins and Lava_Rivers
   TimeOfDay variable names for exact match against supported labels. Add
   aliases **only after source confirmation**, not by blindly fuzzy matching.
2. Verify `Fog (bottom/top)` density and height semantics and the engine
   distance unit scale against the running Evolve build; avoid copying values
   from a later CryEngine version and calling them equivalent.
3. Compare fog at matching map, camera position, horizon angle, altitude,
   time-of-day and weather in game vs Level Editor (screenshots plus
   numeric region comparisons).
4. Replace preview calibration with validated Evolve extinction and
   in-scattering logic. Add source-authored top/bottom fog colors, radial
   scattering, fog-volume entities, shadow modulation, sky and transparent
   interaction. Introduce per-effect toggles/diagnostics so missing
   dependencies fail closed rather than creating fake scene parity.
5. Run `cargo fmt --all -- --check`, full core and Windows desktop CI,
   strict Clippy, Naga shader parse, and live GPU viewport checks before
   any merge or nightly.

Reference: Crytek's CryEngine 3 fog docs describe exponentially decaying
vertical aerosol density and its view-ray integral; this motivates the
model shape **but does not prove** the customized Evolve Stage 2 shader.
