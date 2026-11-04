# Level FBX source-material parity

This PR extends the legacy *geometry-only* level intermediate with
source-authored material, original placement, terrain and local-light data. **It does not yet establish complete level
export parity.** Do not use the existence of an FBX file as proof that every
level mesh, terrain material, or texture was exported.

## Implemented in this branch

- Preserve the existing world-Z / Blender basis node, source UV0/normals,
  separate placement nodes, and original level metadata.
- Key reusable geometry by **model and material override**, not model alone.
  Different placement-level material overrides must not share an incorrectly
  colored mesh.
- Read original CryEngine MTL leaf slots where the exact source can be verified.
- Preserve source subset-to-material binding where the original mesh subset
  table is valid and contiguous. Only assign a default leaf when an MTL has
  exactly one leaf; unknown multi-leaf binding remains unassigned.
- Embed supported authored DDS maps as original-dimension PNG images within
  the GLB handed to Blender. Diffuse, normal, and emissive glTF mappings are
  written, while additional authored channels are retained in glTF extras.
- Preserve original referenced texture paths and non-PBR CryEngine shader
  metadata in material extras.
- Report the count of material slots, embedded images, material binding gaps,
  and unresolved DDS references.
- Add synthetic regression tests for single-leaf mapping, ambiguous multi-leaf
  handling, placement instancing, and missing referenced geometry.

## Further implementation added

- The Level Editor's FBX action now accepts the complete already-decoded map
  scene, rather than reparsing just the selected XML file. This includes
  original VegetationMap.dat placements where a verifiable mesh path exists.
- Original layer XML is re-read from the selected extraction root with canonical
  containment and regular-file checks before extracting light properties.
- Exact-build imported Asset Library resolution now serves models, MTLs and
  authored DDS references across PAKs from the selected installation. Unknown
  or conflicting virtual paths fail instead of choosing another build.
- Authored local Light/LightEntity records are matched by source GUID and
  emitted through glTF KHR_lights_punctual as point/spot lights, preserving
  original color, radius, intensity, bulb size and position. Fake/disabled or
  incomplete sources are skipped. glTF photometry is **not** CryEngine parity.
- Blender's FBX conversion now selects LIGHT objects, exports them as native
  editable lights and checks light counts on reimport.
- Heightmaps are decoded at the decoder's maximum supported 2049-sample
  grid, then split into UV- and world-space-preserving 256-quad tiles, rather
  than reducing all maps to 257 samples. A separately verified, map-owned
  CRY TerrainTexture.pak supplies baked color when supported. Source stride
  is reported; a 4096-source map can still be sampled.
- Blender conversion creates dedicated parent empties for multi-material
  placements before material splitting so all resulting parts can be moved
  together; reimport checks the placement-group count.

- Verified world CGF render parts are decoded independently, each with its own
  source SUBSETS material IDs, then parented under one editable actor placement.
  Physics-only node geometry is excluded by the existing world model decoder.
- Blender FBX conversion keeps the original multi-mesh actor parent empties
  selected, along with the multi-material split parents, and checks their
  survival after reimport.

## Additional source-backed shader, terrain and geometry work

- Streamed DDS .dds.0 and complete .dds.1..N fragments are resolved using the
  exact selected imported game build. The DDS decoder enforces contiguous mip
  sets and dimensions; no low-resolution fallback silently claims full quality.
- BC5 normal-map RGB PNGs reconstruct positive tangent-space Z from the
  original two channels; authored X/Y are unchanged. Y orientation remains an
  in-game validation gate.
- Preserve additional authored MTL fields (source GenMask, opacity/alpha test,
  diffuse/specular tints, glow/ALPHAGLOW, shader params, original role paths,
  decal and diffuse TexMods). glTF material names are unique by material
  identity. Static diffuse tiling is expressed via KHR_texture_transform;
  source specular uses KHR_materials_specular and overbright emissive factors
  use KHR_materials_emissive_strength. All are **cross-renderer mappings**,
  not Evolve shader equivalence.
- When readable Heightmap.dat paint exists, the entire original categorical
  LayerId bitmap is preserved as nearest-filtered lossless PNG (red byte is
  original ID). Each terrain tile's triangles are grouped into materials
  by their source categorical paint ID. The material keeps the original Layer
  name, type, filter, brightness and tiling information; original Terrain.Layer
  MTL paths and texture roles are resolved when unambiguous.
- Painter grouping is **face-center classification only**. The baked source
  terrain atlas remains the visible base; native layer transitions and high-pass
  lighting are not faked.
- Compiled CrChF MtlName declarations (0x08021014) now take precedence
  over a same-stem material when exactly and safely resolvable; conflicting
  authored references are rejected. Case-folded basename search is prohibited.
- Untextured original static meshes without a UV0 stream may be retained
  without inventing UVs. Image-sampled meshes still require authentic UV0;
  disabling tangent generation for mixed UV-less scenes is explicitly
  reported, not silently called shader parity.
- Original object names are kept for level export. Joining all scene meshes
  is disabled for levels (unlike the optional character FBX combine command).

## Remaining parity gaps (keep this PR in draft)

1. **World geometry:** Verified multi-mesh CGF subnodes now have grouped
   placement hierarchies, but wildlife spawns without a verified render path
   remain metadata only. Unsupported geometry, skeletal/compound objects and
   source mesh part reimport counts need real-map acceptance tests.
2. **Vegetation:** existing original binary vegetation transforms are included
   when opening an imported map, but they still depend on verifiable model
   resolution and require Blender placement-count tests.
3. **Terrain:** 4096-source heightmap resolution remains capped by the core
   decoder's 2049 grid. RGB24 map-owned color tiles and the native
   high-pass/detail/height-assisted transition shader are still unresolved.
   Paint IDs and authored layer inputs are preserved, **not** shaded 1:1.
   Face-center material categorization alone cannot reconstruct subtriangle
   transitions.
4. **Shader parity:** complete mip sets and BC5 blue reconstruction are now
   implemented in the source-export path, but renderer-specific normal Y,
   embedded FBX images for every extra map role, gloss/opacity packed channels,
   CryEngine glow-pass interaction, animated UV and alpha blending still
   require verified conversion and actual game-vs-Blender tests. Not all glTF
   extras are retained in FBX even when original source paths remain visible.
5. **Round-trip verification:** CI builds and script regressions are necessary
   but do not substitute for actual Blender 4.2+/5.x reimports of Tutorial and
   Ice_Crashsite, comparing all meshes, material slots, texture resolution,
   vegetation, terrain and independently movable local lights.
6. **Usage scope:** complete multi-layer / cross-PAK mode is reached from a
   currently opened imported Level Editor map. The standalone CLI/source-XML
   export still uses its single selected source and exact local root.

## Blender object-editing contract

- **Do not combine the level into one mesh.** Each original level placement must
  produce a separately selectable Blender object with its own transform.
- Repeated models may share source geometry, but moving, rotating or scaling
  one instance must not transform any other instance.
- Keep object names and source GUID/layer identifiers traceable. If the FBX
  conversion separates a multi-material mesh into multiple Blender objects,
  preserve a per-placement parent/group so the complete prop can be moved as
  one unit.
- Terrain must be a separately selectable object or organized tile collection;
  vegetation must preserve independently movable instances rather than a
  single fused foliage mesh.
- Blender reimport acceptance must alter a sample repeated prop's transform,
  verify no other instance moves, and verify all material pieces move together.
  GLB node-count tests alone are insufficient for FBX acceptance.

## Blender reimport and actual map acceptance (November 4)

- Every Level Editor FBX export now performs a **forced Blender FBX
  round-trip** rather than skipping reimport when no static CDF basis or
  original material tag was present.
- The bridge compares each source-to-FBX mesh count, triangle count,
  original UV0 presence and world-coordinate bounds; it also checks every
  original local-light position. A sample imported mesh is actively moved,
  its independently movable sibling checked, and the transform restored.
  Dropped/fused meshes, missing UVs and displaced geometry fail the export.
- The checks operate on the already-loaded level scene, **not** an in-game
  rendering. Shader colors, material masks, native sun/shadow energy, texture
  visual quality and real gameplay placement remain outside these assertions.
- The exported Level Editor scene can now pass its **original verified
  baked-terrain image** directly to the exporter. The source is the loaded
  map's unmodified RGBA baked terrain surface, not the CPU/GPU high-pass
  preview or a guessed `TerrainTexture.pak` path. This also supports original
  RGB24 tile sets. The image is embedded with all source pixels and dimensions
  preserved; a regression test decodes the image from the GLB and compares
  every pixel. If there is no verified source image, no substitute color is
  synthesized.
- The user's original Tutorial source archive independently contains
  `LevelData/Heightmap.dat` (512 x 512) and `TerrainTexture.xml` describing
  four 1024-pixel RGB24 tiles (2 x 2, 2048² baked color). These inputs and
  the user's separate four raw tiles support source-format inspection but are
  **not** embedded in this repository or CI.
- This execution environment does not contain Blender or Rust toolchains
  or an in-game Evolve renderer. GitHub Actions can check compilation and
  pure regression tests, but a real Tutorial/Ice_Crashsite FBX export and
  paired Evolve screenshots must still be run in the owner's verified
  Windows installation. Do not claim that synthetic test pass equals native
  visual parity.
- The owner-provided shader packages are opaque to ordinary ZIP inspection,
  so their engine-specific native shader mathematics cannot be proven from
  those bytes alone. The exported glTF/FBX materials remain an explicit
  best-effort representation of verified source fields.

### Real-map verification sequence

1. Open the exact-build imported Tutorial map with its terrain texture PAK
   and source MTL/DDS archives. Export a new FBX in the Level Editor.
2. Confirm the exporter completes the Blender reimport audit, preserving
   counts, independent object transforms, point/spot lights, all verified
   UV0 spans, and the original 2048² baked RGB image.
3. Confirm the original LayerID PNG sidecar retains the exact categorical
   source bitmap and is not mistaken for a shaded texture.
4. Compare Blender with a paired Evolve Stage 2 screenshot at matched
   location, daylight, weather, and exposure. Track native normals, POM,
   terrain blend weights, and local-light attenuation as separate gaps.
5. Repeat on Ice_Crashsite with its different 4x4 map-owned tile layout and
   4096² baked atlas. Inspect unsupported geometry and missing source assets;
   do not silently substitute geometry or textures from another build.

## Acceptance requirements before ready for review

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --all-targets`
- Blender export/reimport on representative Stage 2 maps with exact-build PAKs,
  reporting missing assets; never silently substitute textures.

No game files, game PAKs or proprietary SDK binaries should be committed.
