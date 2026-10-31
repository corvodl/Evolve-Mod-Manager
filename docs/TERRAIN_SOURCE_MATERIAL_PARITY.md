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

The current Level Editor uploads the baked atlas, but its world shader has
**no CryEngine terrain detail-material pass**. The missing material pass is
a format-level limitation shared by every map using these original terrain
sources. Fixing only Tutorial-specific colors would hide the problem.

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

These source references are diagnostics **only**. They are not yet
resolved to exact installed-build `.mtl` and DDS files or blended in the
shader. The unlit switch is a comparison tool, **not** a replacement for
the missing native high-pass terrain detail material pass. The renderer must not fabricate terrain detail, default to a
different game build, or call this native parity without validating Evolve.

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

1. Resolve each authored `SurfaceType` to an **exact matching installed
   Game directory and build**, including complete source `.mtl` and DDS
   dependencies, with precise reasons for missing/ambiguous members
2. Determine authentic CryEngine/Evolve terrain detail blending (UV projection,
   high-pass composition, authored tint/brightness, transition weights,
   surface type ID semantics and slope/altitude behavior) from original
   source + matched in-game captures. Do not invent a multiplicative blend
3. Add unlit macro-color and final shaded comparisons so axis/channel/
   gamma mistakes can be distinguished from missing material data
4. Validate Tutorial **and** at least one second map that has different
   layer IDs, tile count or RGB/CRY atlas format. Keep rendering efficient
   for all supported terrain layouts

The expected result is a reusable terrain material renderer, not a
single screenshot color patch.


## Supplied PAK provenance fixtures (2026-10-31)

The two privately provided ZIPs are named for their **original source PAKs**, not map-owned texture files. Never rename them to terraintexture.pak or treat the ZIP files themselves as DDS resources:

- \`materials_min_terrain.zip\` represents \`materials_min.pak\` terrain members: **90** \`materials/terrain/*.mtl\` source definitions. Their \`Shader="Terrain.Layer"\` slots preserve original \`PublicParams\`, texture roles, \`TexMod\` tiling and \`SurfaceType\`. Referenced \`.tif\` files are authored names; normal import resolution selects cooked \`.dds.0\` followed by a complete split-mip set.
- \`materials_basic_terrain.zip\` represents \`materials_basic.pak\` terrain members: **3** high-pass diffuse source sets, each split into \`.dds.0\` through \`.dds.6\` (**21** archive members). Their DDS headers report **1024×1024 DXT1**, and the higher-resolution blocks reside in numbered fragments. Do not attempt to render the tiny low-resolution \`.dds.0\` tail as the full high-resolution image or claim it is complete without the other parts.

The same-install diagnostic now attempts an exact, verified material lookup for every original declared layer/surface material and audits each known terrain texture reference through the existing split DDS inspector. No game bytes or user ZIP assets were committed. Archive labels are secondary to the original installed \`Game\` root; differing known build labels are excluded. Missing and ambiguous dependencies are reported explicitly, and unrelated basename matches are forbidden.

**Important limitations:** These extracts do not supply most referenced bump, height, and specular textures. The present additions are source/dependency verification and debugging, **not** native terrain detail shading: the GPU still does not evaluate \`Terrain.Layer\` high-pass composition, masks and transitions. Native color parity requires same-build complete dependencies and visual comparisons with the running game. A PAK's filename alone cannot prove source build compatibility when importing fixtures from an unknown installation.
