# Terrain material provenance and rendering parity

## Why the baked terrain atlas alone is not enough

A valid CryEngine Heightmap.dat contains two different kinds of terrain data:

1. A height grid and optional per-cell *LayerId* bitmap.
2. Authored `<Layer>` definitions with `SurfaceType` material paths,
   `FilterColor`, `LayerBrightness`, and `LayerTiling`.

The map-owned `TerrainTexture.xml` / RGB24 tile files encode a separate
baked **macro-color atlas**. This image is not necessarily final shaded
terrain. Flat regions can be legitimate source pixels. In particular a
constant `[128,128,128]` block does not prove GPU streaming failed.

The Level Editor now implements an **experimental source-driven Terrain.Layer detail preview**. It preserves the baked atlas and composites original high-pass DDS data over it. The original Evolve terrain layer-mask equations, source project-axis handling and native-distance transitions remain unverified; this cannot yet be described as a 1:1 port of the game renderer.

## What this PR implements

- Decode and retain each original authored `Layer`'s ID, label,
  `SurfaceType` material path, tint, brightness and tiling, without
  hardcoded map or material names
- Decode the **separate** `<SurfaceTypes>/<SurfaceType>` table, preserving
  `SurfaceTypeID`, `DetailMaterial`, `DetailTexture`, tiling scale and
  projection axis. Never join those IDs numerically to painted `LayerId`:
  the user's real Tutorial has Layer IDs such as 22, 23 and 27 while its
  source SurfaceType IDs run from 0 through 9
- Preserve duplicate Layer IDs rather than silently collapsing all
  authored entries to a single material; report conflicts
- Report the number of bitmap IDs with declared source materials,
  unmapped/special IDs, and sample material paths in Level Editor diagnostics
- Add a **Terrain: Source RGB (unlit)** switch in the Scene tab. This
  renders only the original ground-color atlas without preview sunlight,
  fog, material BRDF or exposure; foliage, water and other models retain
  their existing shading. Toggle it at the same camera position to isolate
  source imagery from the preview renderer's lighting contribution
- Add synthetic source-format regression tests; no CryEngine SDK, original
  PAKs or game texture bytes are checked in

The source metadata is now used for actual build-scoped MTL/DDS loading when the referenced files are imported. The unlit switch intentionally remains an exact source-image diagnostic. The high-pass path uses the SDK-documented add-signed composition, but the boundary weight, displacement bias, world-space tiling and distance behavior are preview approximations until verified against the actual Evolve shader.

## User-supplied Tutorial fixture observations (not committed)

The supplied four RGB24 tiles each start with two little-endian `u32`
dimensions of 1024 × 1024, followed by exactly 3,145,728 RGB bytes.
They form one complete 2×2, 2048 × 2048 macro-color atlas.
Large constant-color regions are present *in the source*, including
`[128,128,128]` and `[66,73,69]` patches.

The accompanying Heightmap.dat declares 512 × 512 world-height samples
and multiple authored `Materials/terrain/*_highpass` surface materials,
plus per-layer colors and brightness. TerrainTexture.xml independently
describes the 2×2 tile grid. The ZIP does not contain the referenced
material definitions or their original high-pass textures. The source
atlas and terrain layer metadata are therefore **insufficient to reconstruct
native ground shading without the referenced assets**.

These observations provide a real-format acceptance case, but the decoder
and UI must handle arbitrary supported tile grids, Heightmap dimensions,
material names, IDs, and layer count. No Tutorial-only constant or codepath
is allowed.

## Next implementation gates

1. **Implemented in preview:** resolve authored paint-layer material paths from imported `.mtl` and cooked DDS streams under the original installed Game root, diagnose absent/ambiguous references, and fail closed on incomplete mip chains. Complete game inputs must still be supplied.
2. **Partially implemented:** authored `Terrain.Layer` diffuse and `TexMod` tiling, optional height-based transition adjustment using `BlendFactor`/`BlendFalloff` and `DetailTextureStrength`, and add-signed high-pass WGSL shading. **Still requires native validation:** source transition masks, UV projection/DetailScale, POM/OBM, slope rules, normal/specular detail behavior and accurate distance thresholds. The interpolation and height transition are explicitly preview-only.
3. **Implemented in preview:** unlit macro color vs shaded high-pass detail. Add capture-based per-pixel native Evolve comparisons to verify axis/channel/gamma.
4. Validate Tutorial **and** at least one second map that has different
   layer IDs, tile count or RGB/CRY atlas format. Keep rendering efficient
   for all supported terrain layouts

The expected result is a reusable terrain material renderer, not a
single screenshot color patch.


## Supplied PAK provenance fixtures (2026-10-31)

The two privately provided ZIPs are named for their **original source PAKs**, not map-owned texture files. Never rename them to terraintexture.pak or treat the ZIP files themselves as DDS resources:

- `materials_min_terrain.zip` represents `materials_min.pak` terrain members: **90** `materials/terrain/*.mtl` source definitions. Their `Shader="Terrain.Layer"` slots preserve original `PublicParams`, texture roles, `TexMod` tiling and `SurfaceType`. Referenced `.tif` files are authored names; normal import resolution selects cooked `.dds.0` followed by a complete split-mip set.
- `materials_basic_terrain.zip` represents `materials_basic.pak` terrain members: **3** high-pass diffuse source sets, each split into `.dds.0` through `.dds.6` (**21** archive members). Their DDS headers report **1024×1024 DXT1**, and the higher-resolution blocks reside in numbered fragments. Do not attempt to render the tiny low-resolution `.dds.0` tail as the full high-resolution image or claim it is complete without the other parts.

The same-install diagnostic now attempts an exact, verified material lookup for every original declared layer/surface material and audits each known terrain texture reference through the existing split DDS inspector. No game bytes or user ZIP assets were committed. Archive labels are secondary to the original installed `Game` root; differing known build labels are excluded. Missing and ambiguous dependencies are reported explicitly, and unrelated basename matches are forbidden.

**Important limitations:** These extracts do not supply most referenced bump, height and specular textures. The implementation now reconstructs original diffuse high-pass overlays and applies add-signed composition in WGSL, with categorical layer-ID interpolation and an optional bounded height-adjusted blend where original displacement maps are available. It **does not** reproduce Evolve's exact POM/OBM, triplanar/project-axis blending, normals, specular, or native transition weights. Native parity requires same-build complete dependencies and paired in-game captures. A PAK filename alone cannot establish source build compatibility.

Reference: [CryENGINE 3 Terrain.Layer shader](https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1114113/pages/1048772) and [Painting Terrain](https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1114113/pages/1048843) describe add-signed high-pass diffuse, baked distant color, authored detail strength, and height-assisted transitions. They are **SDK-level references**, not proof of the exact Evolve runtime shader.


## Tutorial screenshot regression (October 31, 2026)

The reported screenshot of Tutorial after PR #165/#166 still shows angular gray terrain and missing source ground detail while more world props have resolved materials. The imported terraincolor source is a 2×2 RGB24 tile grid assembled into a 2048×2048 atlas and applied successfully; that fact does **not** establish correct terrain appearance. At least 9/10 terrain MTLs and 27 DDS headers were discoverable, yet the high-pass composer returned zero usable layer images. The initial repair is therefore to unify the composer's MTL diffuse source with the verified `read_material` leaf parser rather than trusting an independent XML scan. It also accepts original CryXmlB MTL for optional shader parameters, and records the specific source/DDS failure on a zero-detail result.

**Acceptance for this fix:** On the same imported Tutorial build, inspect the new per-layer error message and compare unlit original RGB ground with shaded ground. Confirm which of the 9 declared active IDs reconstruct full-resolution DDS images, while retaining the 5 unmapped/special bitmap IDs without guessing their materials. If the gray polygons are already present in unlit RGB, the remaining root cause is baked tile provenance/layout, atlas projection or missing original layers, not the high-pass shader. Do not replace original gray pixels with guessed browns or greens or declare Evolve parity based only on CI.

## Source textures.pak inventory (2026-10-31)

The owner-supplied `textures.zip.001` through `.004` are four sequential parts of **textures.pak**, not four original game PAKs. Read-only inspection of their combined ZIP central directory showed **7,844 entries** containing **1,562 `textures/terrain/` entries**, **2,612 `textures/particle/` entries**, and further source decal/road/sky textures. The ZIP parts are test references only and are **not** packaged in Git or the installer.

- Original `materials_min_terrain` MTL references matched **229** `textures.pak` cooked terrain DDS heads (diffuse, normal, displacement, specular); **46** references target different original sources, including map-specific level texture PAKs and special high-pass material paths. Thus `textures.pak` fills a major but **not complete** terrain dependency set.
- The `materials_min_particle` definitions matched **832** authored particle texture references in `textures.pak`; `objects_min_effects` definitions matched **835** additional particle texture references. Other refs remain external. These counts are authored references, not unique images or executable particle definitions.
- `textures/terrain/arctic/snow_dirt_a_terr.dds.0` and `ice_a_terr.dds.0` each report **1024×1024 DXT1 with nine mips**. Each `.0` head stores only a small 296-byte mip tail, while the highest-resolution block resides in `.6` (524,288 bytes), with `.1..5` in between. Likewise `textures/particle/light/glow_prism_01a.dds.0` uses **1024×1024 DXT5**, and `.6` supplies 1,048,576 bytes of high-resolution blocks. Reading the `.0` tail by itself cannot recover full source quality.
- A concrete bug in PR #168's original high-pass loader was that it passed every locally indexed `.dds.N` to `decode_dds_rgba_with_parts` as an **additional external fragment** even though that decoder scans the `.dds.0` parent directory itself. The same fragment was discovered twice, and decoding returned a duplicate-fragment error. The patched loader now passes **only fragments from different extracted PAK directories**, while verifying the source manifests and leaving same-directory numbered mips to the decoder. A synthetic same-directory/external path regression test protects this behavior.
- New map diagnostics enumerate **exact original same-build** `textures.pak`, `materials_min_particle.pak`, `objects_min_effects.pak`, and `objects_basic_effects.pak` imports and show their DDS head/material/mesh counts, so the user can immediately tell which dependencies are absent from the Asset Library.

These findings explain why Ice_Crashsite can load its complete baked 4096×4096 macro color atlas while still appearing blurry nearby: without the matching high-detail split DDS chains, the terrain cannot show source-authored fine texture. The patch repairs the duplicate-fragment decoding failure, **not** Evolve-native terrain projection/blend formulas. Visual acceptance must still check full-resolution detail on the user's imported Stage 2 assets and compare the Lit and Source RGB modes.

## In-game Tutorial comparison and atlas minification (November 1, 2026)

A paired Level Editor / Evolve Stage 2 Beta screenshot shows bright cyan-colored
terrain speckling in the editor versus dark, physically shaded earth in game.
The original Tutorial map-owned `Environment.xml` records
`DetailLayersViewDistRatio="0.15"` and a map color-chart reference
`levels/Tutorial/colorgrading/tutorial_color_grade.tif`. The present viewer
does **not** evaluate that LUT, CryEngine's distance-dependent terrain pass,
terrain bump/displacement/OBM/POM, authored surface shadows, or native
light-adaptation. It must not claim visual parity on the basis of one fix.

A separate deterministic failure was located in the existing CPU high-pass
compositor: every map-wide atlas pixel bilinearly sampled exactly one texel
from the full-resolution, repeating material diffuse. A 512-wide paint map,
2048-wide baked atlas, authored tile scale 0.25 and 1024-wide source DDS
represent about **64 original texture texels per output pixel**. Without
minification filtering this produces incorrect low-frequency false colors
and severe aliasing instead of real ground detail.

PR #171 constructs minified source-image mip levels once per material,
selects a trilinear LOD from the *source texture footprint per atlas pixel*,
and retains existing categorical paint coverage and source material
provenance. This is an anti-aliasing correctness fix, not a restoration of
high-frequency texture detail: a map-wide CPU atlas can never contain the
full per-pixel detail of a repeated 1024² material everywhere. **Full native
near-camera terrain quality requires moving authored Terrain.Layer
diffuse/normal/displacement sampling into the GPU world pass and reconstructing
the original layer masks, projection and distance behavior** rather than
sampling those materials into the macro-color atlas.

Acceptance requires paired positions and matching time-of-day under Tutorial
and Ice_Crashsite, separately checking: original unlit baked RGB; material
layer IDs/UV orientation; close-up DDS high-frequency texture; native
shadows/normal mapping; and color-chart/postprocessing. No arbitrary brown
tint, swapped-channel heuristic or borrowed map texture is permitted.

## GPU original terrain diffuse sampling (2026-11-01)

The native Tutorial screenshots now cover canyon interiors, climbing cliffs, damp soil, waterfalls, shoreline and orange-hour sky. They are comparison targets, not proof of renderer parity.

The old CPU detail overlay lost high-frequency source DDS detail by resampling repeating materials into a single baked map-wide atlas. The new GPU path retains up to 16 same-build verified equal-dimension Terrain.Layer diffuse sources (2048-pixel per-axis and 256-MiB raw base cap), uploads their full decoded pixels and mip chains into a 2D texture array, and fetches exact original categorical LayerId bytes in a separate paint map. Per-fragment sampling honors authored TileU/TileV and DetailTextureStrength over the original unmodified baked ground colors. Material arrays are included in GPU residency diagnostics. Unsupported arrays retain the mip-filtered CPU fallback.

The four-neighbor category interpolation is an approximation, not Evolve's original splat masks. Normals, height/POM, slope projection, specular lighting, game shadows, native exposure, reflections and water remain outstanding. Real Tutorial and Ice_Crashsite Windows screenshots at matched positions, plus shader validation and fallback tests, are required before parity claims.
