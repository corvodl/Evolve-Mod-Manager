# Imported Level Editor — PR #130 (experimental)

## How to open

1. Import your original Evolve level PAKs and their dependency PAKs into Asset Library.
2. Select an archive categorized as **Levels**. The manager opens its extracted level in **Level Editor** automatically, without creating an editable copy. Use **Tools → Open in Level Editor** if the PAK name is unconventional.
3. If the archive contains multiple named maps, choose a map inside Level Editor. The perspective viewport shows the decoded terrain and any compatible authored object meshes. The Scene Outliner lists placements. Use the camera presets, mouse orbit/pan, and scroll zoom. Expand the optional 2D placement overview for point selection.
4. To make changes, choose **File → Open as Modding Project**. This routes the current source PAK through the existing verified editable-project copy process. The original imported archive is never edited.

## Loading contract

The importer revalidates the selected archive's manifest. Original `Mission.xml`, `.lyr`, and `Heightmap.dat` are selected by their **exact extracted virtual paths**, not approximate filename searches.

For authored geometry referenced by the parsed level objects, the read-only scene worker indexes verified imports from the same unambiguous build snapshot. A single exact virtual-path match is usable; duplicate matches across unrelated archives are diagnosed as ambiguous. The original mesh decoder and material subset loader resolve source geometry, compiled material maps, diffuse/normal/emissive/specular/gloss/opacity textures where supported, and source world transforms. Terrain and all resolved model instances share one wgpu preview mesh with material ranges and depth testing.

Missing assets, unsupported formats, and scene-budget limits are reported rather than replaced with invented props or textures.

## Still not equivalent to Unreal Editor or the private Level Editor

- The perspective viewer currently uses the existing CPU-projected wgpu mesh pipeline, **not native GPU instancing, free-flight WASD navigation, selection gizmos, or a scalable streaming world renderer**.
- Compiled vegetation/foliage instance streams and authored terrain splat materials are **not decoded**. Any foliage represented as standard renderable level placements may display, but this is not a complete vegetation import.
- Original terrain is currently geometrically decoded with a bounded sampling stride, not the full-detail terrain and original splat layer set.
- Cross-PAK assets require unambiguous build provenance and verified original virtual paths. The current per-scene reader has explicit bounds on indexed archives, distinct models, instances, vertex/index counts, and material surfaces; huge maps can be partially rendered with diagnostic notices.
- Hierarchy editing in the Level Editor is only implemented for the legacy local-file layer path. The imported scene intentionally cannot be edited in place. **File → Open as Modding Project** opens the existing modding workspace copy, not a completed 3D edit-and-save flow. Bringing editable projects back into this viewport and rebuilding their modified scene on every edit remains outstanding.
- Other unsupported level data includes terrain shader layers, dynamic script entities, lighting probes, effects, and game runtime behavior.

## Required acceptance before merge or nightly

Run `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace --all-targets`.

With a real imported Stage 2 map, verify:

- Selecting a level PAK auto-opens a map without manual file paths; selecting an ordinary texture PAK does not.
- Both terrain and multiple geometry instances are rendered in the **same** perspective viewport with correct world transform and depth.
- Original diffuse/normal maps appear where exact source material dependencies are present in a matching imported build; missing and ambiguous references produce diagnostics.
- Opening a map and navigating the viewport never modifies an imported asset. Existing library source files retain hashes.
- Creating a modding project from the Level Editor File menu creates a separate editable copy and leaves originals untouched.
- Large-map memory use, viewport drag frame rate, and build-scoped loading responsiveness meet the project's acceptance standard. A green CI build alone does not establish this.
