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

## Editable copy in the level viewport (new experimental path)

Open **Modding → select project → Edit Level** (or **Edit → Open Level Editor**) to reopen the original editable project's extracted Level/LYR and terrain sources in the same viewer. A project created by **File → Open as Modding Project** from a read-only map is reopened automatically after indexing.

In an editable session, select a mesh by clicking an actual projected triangle in the 3D viewport. A selected object's red/green/blue axis guides provide drag-to-translate on X/Y/Z. The transform inspector also supports precise translation, rotation and scale, Undo and Redo. The in-memory mesh preview is updated for the affected source object index ranges and normal vectors, without changing the underlying imported PAK.

**File → Save Edited Project Layer** exports the changed XML to an isolated temporary file, checks the original source digest, and uses the existing managed Modding import transaction to back up and replace only that registered editable project member. The original Asset Library never changes. This command only allows one LYR/XML source layer and at most 128 pending edits; binary vegetation instances are read-only. Use Build Mod from Modding after a successful edit to produce the package.

The picking routine is currently a **bounded source-triangle screen-space test**, not a GPU ID buffer or BVH. Huge levels may hit its triangle test limit and miss objects. The selected transform axis guides and viewport must be verified visually on Windows.

## VegetationMap.dat format and decoder

The editor now searches for a separate **LevelData/VegetationMap.dat** alongside normal layer and heightmap assets. For legacy CryEngine Sandbox archives, this file contains an MFC-length-prefixed serialized VegetationMap XML definition, followed by CNamedData blocks. It recognizes the named \`VegetationInstancesArray\` block and the explicit CryEngine packed record variants:

- **Version 1:** 19 bytes (xyz Float32, scale Float32, u8 prototype ID, u8 brightness, u8 quantized yaw).
- **Version 2:** 30 bytes (xyz Float32, scale Float32, u8 prototype ID, u8 brightness, three Float32 angles).
- **Version 3:** 33 bytes (xyz Float32, scale Float32, i32 prototype ID, u8 brightness, three Float32 angles).

Objects map to author-defined \`FileName\` .cgf paths in the same XML, then enter the normal verified cross-PAK geometry/material resolver as read-only placed instances. The decoder validates bounds, finite transforms, object indices, path safety, version and packed record stride, and handles zlib-compressed named blocks when the stream matches this format. Unknown or nonconforming Stage 2 variants produce an explicit diagnostic, not guessed objects.

**Format source:** the original licensed CryEngine Sandbox/Lumberyard \`CVegetationMap::Serialize\`, \`SerializeInstances\`, \`CNamedData::Serialize\` and \`CArchive::readStringLength\` implementations. The format is grounded in source, but the live Evolve Stage 2 variant still requires real-map validation before marking full vegetation support.

## Performance and shader limitations remain

The world viewport still uses the existing CPU-projected, textured wgpu pipeline and a bounded combined mesh. The new triangle picker and in-memory transform updates do **not** constitute a dedicated scalable GPU-instanced or streamed scene renderer. A real world renderer must retain one GPU mesh per unique source model and use per-instance GPU transforms, camera frustum culling, spatial indexing/BVH, geometry/texture LOD and streaming; frame-time and memory tests on real multi-PAK Stage 2 maps are required. Lighting, terrain splats, physics and CryEngine vegetation wind/material shaders also remain unsupported.

## Terrain atlas GPU upload and duplicate-layer recovery (PR #156)

The original Tutorial baked atlas is 2048×2048 (16 MiB RGBA8); Ice_Crashsite
is 4096×4096 (64 MiB RGBA8). Previously the Level Editor reported that both
were decoded and assigned to an original terrain material, **but** the world
GPU uploader silently rejected any diffuse larger than its general 32 MiB
model-material budget. Ice_Crashsite therefore sampled a one-pixel fallback.

The terrain world's nonselectable ground group now uses an independent
64 MiB source-atlas budget, subject to the GPU device's maximum supported
2D texture dimension. Other original model textures retain their 32 MiB
budget. If they are bigger, they are **downscaled with their RGBA mask**
instead of being replaced by a solid fallback. Normalized terrain UVs use
clamp-to-edge sampling while prop materials retain repeat sampling.
No source file is changed and no texture is borrowed from another build.

CryEngine layers sometimes repeat identical object GUIDs. A new read-only
combined scene-recovery parser retains one source-ordered declaration,
records conflicts, and resolves parent transforms across the entire
validated set of layers. Conflicting GUIDs may not serve as parents, so
an ambiguous placement is not used to move children to an arbitrary
coordinate. The strict editable scene parser still rejects all duplicate
GUIDs. If combined recovery cannot validate the sources, the previous
per-layer fallback remains available.

**Remaining source-data limitations.** The screenshot logs also identify
58 (Tutorial) / 122 (Ice_Crashsite) model-material pairs with no verified
diffuse and explicit missing original vehicle DDS candidates. These files
are *not* made available by changing GPU upload limits; the exact selected
build still must supply the correct cooked DDS. Missing textures must remain
explicitly diagnosed, rather than substituted with similarly named images.
An interactive Windows screenshot comparison of the actual maps is still
required to confirm terrain orientation and the visual outcome.

## Source-authored time of day, particle libraries, and shader discovery (PR #156)

The editor now searches the verified selected map and same-build imports for
`LevelData/TimeOfDay.xml` by exact authored virtual path. It decodes
CryEngine `Variable` entries and `Spline Keys` (normalized 0–1 clock)
and uses the original sun color, sun intensity, sun color multiplier, and
recognized sky/ambient values while scrubbing Scene Settings. XML remains
untouched. If unsupported or missing, it reports approximation mode.
Original source keyframes are **linearly** sampled for this preview:
CryEngine spline tangents, fog, sky, HDR, GI, probes, light entities,
shadows, and native shaders are **not** reproduced.

Verified selected-import `Libs/Particles/*.xml` libraries, `.mtl` authored
shader assignments and `.cfx` source scripts are catalogued with bounded
name reads, and particle-entity placements are counted. Their presence is
shown in Scene Settings. This imports **source descriptions**, not live
particle simulation or engine-equivalent shader code. Source-dependent
materials continue through the existing wgpu material pipeline. Full
particle rendering, proprietary shader translation, and accurate
environment composition remain separate implementation and acceptance
work. Different-game and unrelated-build fallback assets are not borrowed.

CI and screenshot acceptance on actual Stage 2 maps remain necessary.
