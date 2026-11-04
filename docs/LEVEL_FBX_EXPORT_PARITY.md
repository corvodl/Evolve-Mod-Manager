# Level FBX source-material parity

This PR begins replacing the legacy *geometry-only* level intermediate with
source-authored material metadata. **It does not yet establish complete level
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

## Not yet implemented (do not claim parity)

1. **Level Editor scene assembly:** the FBX action still starts with a selected
   XML/LYR file and an extraction root instead of exporting the exact verified
   multi-layer, cross-PAK, compiled-companion scene in the Level Editor.
2. **Full world geometry:** dedicated vegetation instance arrays, source wildlife
   spawns, complete skeletal and compound objects, and supported collision/
   effect variants need explicit export policies and tests.
3. **Terrain:** Heightmap geometry remains sampled to the existing 257 grid.
   Authored terrain RGB atlas, layer paint IDs, layer diffuses, high-pass
   composition and full-detail mesh export are not in this path.
4. **Shader parity:** DDS normal/BC5 reconstruction, specular/gloss/opacity,
   detail, blend, alpha-glow and animated UV channels need Blender-specific
   material conversion and verification. Material extras preserve provenance,
   but FBX does not necessarily retain arbitrary glTF extras.
5. **Cross-PAK texture discovery:** missing assets must use the selected
   installation's exact verified library index; no guessed basename fallback.
6. **Acceptance:** real Tutorial and Ice_Crashsite exports, Blender FBX
   reimports and comparison of all mesh/material/texture counts, UVs, positions,
   source-resolution images, and terrain are still required.

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
