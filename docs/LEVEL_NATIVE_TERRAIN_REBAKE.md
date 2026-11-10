# Full native terrain rebaking — Level Editor acceptance gate

**Status: BLOCKED — required before claiming complete native terrain-paint authoring.**

This is a required feature of the project-only Level Editor draft (#196, stacked
development #200). It is **not** satisfied by modifying the authored
`Heightmap.dat` `LayerId` bitmap, by exporting categorical sidecars, or by
running a plausible-looking height-based blend. The application must not label
any approximated output as Evolve-native terrain rebaking.

## Existing verified evidence

- `terrain.rs` can edit the source height samples and exact original
  categorical `HeightmapLayerIdBitmap_ver2`. Unknown compiled tail bytes
  are preserved, not reconstructed.
- The original `GetLayerBlendingValue` scalar and
  `pPass.fAlpha = saturate(interpolated_source_channel - AmbientOpPB)`
  expression have been traced from parsed Evolve shader representations.
  `blender_terrain_blend.py` deliberately rejects invented alpha/height.
- Authenticated Tutorial sources include `Heightmap.dat`,
  `terrain/terrain.dat`, four 1024×1024 RGB24 baked tiles and packed
  D3D11 terrain shader variants. Parsed sector surface-type selectors are
  categorical, **not** an authenticated per-pixel alpha mask.
- Packed `CPCK` shader-cache variants are not yet verified as standalone
  disassemblable DXBC; 244 finest compiled sectors were recovered and 12
  were not. Do not extrapolate missing instructions, vertex attributes,
  sectors or blend maps.

See [LEVEL_FBX_EXPORT_PARITY.md](LEVEL_FBX_EXPORT_PARITY.md) for prior evidence,
hash checks and source-bounded intermediate functions.

## Implementation milestones

### 1. Recover original paint alpha and full source bindings — BLOCKED

- Reconstruct the actual provenance and meaning of the packed interpolator
  feeding `pPass.fAlpha`, including vertex input layout, interpolation,
  `AmbientOpPB` binding and texture/sampler bindings from a matching
  **in-game terrain draw**. Derive per-pixel values from authenticated source,
  **never** from neighborhood LayerId interpolation, a surface-type nibble,
  guessed 0.5 coverage or invented Gaussian smoothing.
- Corroborate decoded operations with the selected *same-build* compiled
  `TerrainLayerVS`/`TerrainLayerPS` variant, its input signature, bound
  buffers and actual draw state. If a `CPCK` decoder is written, validate its
  bytecode envelope and checksums first; encountering `DXBC` inside a
  container is insufficient.
- Add small deterministic test vectors from authenticated traces (privately
  held), and fail-closed source-provenance checks in the public implementation.

### 2. Implement full native multi-layer shading — BLOCKED

- Recover actual shader pass/layer ordering, overwrite/accumulate behavior,
  source high-pass/detail color, normal/spec/gloss/opacity handling, height
  bias, tiling/projection axes and original color-space behavior.
- Apply `GetLayerBlendingValue` only with verified shader inputs and the
  correct material for the corresponding authored terrain layer.
- Test multi-layer transitions on at least two original maps, including
  regions with three or more overlapping materials. Compare each layer
  contribution and final pixel values against captured original-game output.
  A one-layer scalar test is **not** full terrain parity.

### 3. Rebuild and package native baked terrain resources — BLOCKED

- Identify the complete native `TerrainTexture.xml` and RGB24 tile descriptor
  mapping (tile addressing, orientation, dimensions, channel order,
  mip/streaming rules and exact archive member paths) for each supported
  map/build. Preserve unchanged tiles byte-for-byte.
- Rebuild affected tiles at original resolution using verified paint weights,
  actual same-build material textures and the genuine compositing operation.
  Do not upsample a categorical preview and represent it as native tiles.
- Re-encode all required modified members while preserving unrelated
  source/compiled data. If a native compiled terrain update is needed,
  establish its exact layout before allowing saves.
- Stage the multi-file change in a **single checked project-only operation**
  with original member checksums, immutable backups, a complete preflight
  and rollback on failure. No write may touch installed original signed PAKs
  or imported Asset Library archives. Refuse incomplete/out-of-date
  source material or mismatched build IDs.

### 4. Acceptance tests / release gate — NOT YET MET

- Synthetic unit tests: boundary tiles, brush locality, array bounds,
  byte-order, unchanged-member identity, duplicate/ambiguous layer IDs,
  source mismatch rejection, partial-save rollback, and deterministic
  rebake output for **verified** supplied pixel traces.
- Optional private read-only fixtures: Tutorial and Ice_Crashsite
  `Heightmap.dat`, matching `level.pak`, baked terrain RGB24 tiles,
  shaders/shadercache, material and texture PAKs. No proprietary asset
  may be checked into the public repository.
- Real maps: edit -> project save -> close/reopen -> rebuild/sign mod ->
  load in the original game -> capture same camera/time-of-day/material
  transition -> compare native output, including normals/roughness and
  texture atlas seam continuity.
- Both Rust CI and Windows desktop CI must pass, plus a successful
  end-to-end in-game verification, **before** the feature is marked
  implemented or the authoring PR is marked ready for review.

## Required additional evidence

The owner previously supplied `shadercache.pak` and Tutorial source PAKs,
but the authentic **per-pixel input path** is still unresolved. The most
useful missing verification is a same-build D3D11 RenderDoc/PIX-style capture
of a terrain material transition with the input layout, vertex buffer
contents, the bound paint/alpha resources and pixel/vertex shaders.
An independently validated original terrain asset compiler/serialization
routine could additionally prove the exact RGB24 tile output format.

Do not silently replace missing evidence with an approximation. Until
these criteria are met, keep the old baked color unchanged during
categorical LayerId authoring, report the visual limitation in the UI,
and maintain this milestone as **BLOCKED**.
