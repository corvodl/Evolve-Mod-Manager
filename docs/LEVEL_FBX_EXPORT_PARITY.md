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

## Remaining parity gaps (keep this PR in draft)

1. **World geometry:** Multi-mesh CGF render nodes need per-part authored
   subset IDs and instanced hierarchy; wildlife spawns without a verified mesh
   path remain metadata only. Unsupported render formats must not be faked.
2. **Vegetation:** existing original binary vegetation transforms are included
   when opening an imported map, but they still depend on verifiable model
   resolution and require Blender placement-count tests.
3. **Terrain:** 4096-source heightmap resolution is still capped by the core
   decoder's 2049 grid. RGB24 map-owned color tiles, layer paint IDs, layer
   diffuse/normal blending, high-pass detail and exact native shader results
   still need separate export handling and source-backed tests.
4. **Shader parity:** DDS high-mip reconstruction, BC5 normal reconstruction,
   specular/gloss/opacity, terrain detail, animated UV and CryEngine glow are
   not yet proven equivalent to Blender's FBX materials. glTF extras alone
   do not guarantee FBX metadata preservation.
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

## Acceptance requirements before ready for review

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --all-targets`
- Blender export/reimport on representative Stage 2 maps with exact-build PAKs,
  reporting missing assets; never silently substitute textures.

No game files, game PAKs or proprietary SDK binaries should be committed.
