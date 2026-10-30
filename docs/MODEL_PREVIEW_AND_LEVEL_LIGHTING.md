# Model preview indexing and level material/lighting follow-up

## Model loading
The Model Viewer now shares the Level Editor's operation-scoped, exact-build
Asset Library manifest index. Instead of repeating full cross-PAK catalog and
manifest reads for every part/material/texture, the model worker builds one
lookup table at the start of dependency discovery, compiled model inspection,
CDF geometry/material assembly, DDS preview, or material/damage material loading.
The scope is thread-local to the blocking worker, so there is no shared mutable
cross-thread index and it is released on both success and failure.

The lookup is **not** a material guessing fallback: the same build tag, exact
authored virtual paths, original source material slots, metadata byte size,
and canonical-path containment rules still apply. When no unambiguous selected
build exists, normal fail-closed lookup remains in effect.

### Acceptance on real imported Stage 2 maps and characters
- Compare first-open and repeated-open latency (CDF characters with many
  attached parts, ordinary CGF and standalone SKIN/CHR). Record total time,
  source geometry time, dependency-index time, DDS decode and GPU upload.
- Verify identical material slots, diffuse/normal/emissive/opacity results,
  UV0, glow values and subset counts before and after the optimization.
- Re-run high-mip DDS and missing/ambiguous/cross-build references; do not
  compensate for absent source files with nearest-basename matches.
- Confirm model viewer progress stays responsive and preview cache remains
  bounded. This change does not itself optimize GPU upload, mesh decoding or
  the renderer's drawing cost.

## Why gray map geometry remains
The Level Editor's **missing geometry** count is not a texture-success count.
An imported CGF can decode correctly with no safe authored diffuse texture.
The scene status now counts unique model/material pairs with no verified
diffuse separately from unreadable models; repeated instances are not counted
as separate texture failures.

Original baked terrain color requires the exact map-owned
`LevelData/TerrainTexture.xml` and `terraintexture.pak` matching the selected
build. A readable terrain layer-ID bitmap is *not* the baked ground image.
If these assets are absent from the imported snapshot, report which is missing
and import the appropriate source PAK; never borrow another map's atlas.

Still needed for full original materials:
- Inspect each missing model/material pair's original MtlName, mesh subset
  table and referenced MTL leaf; report missing, ambiguous, source-only
  and unsupported formats separately.
- Resolve additional DDS dependencies only from the exact selected build.
- Add validated multi-part / multi-material subset mapping rather than
  painting multi-leaf materials into an unknown slot.

## Map lighting feasibility
CryEngine 3 levels may store authored environmental and time-of-day parameters
in `LevelData/Environment.xml` and `LevelData/TimeOfDay.xml`, plus local
light/probe entities in mission or layer XML. These must be discovered by the
exact map's virtual directory and build, not by matching map names globally.

Planned stages:
1. Read-only inventory and XML decode; report lighting assets and source
   light entities with raw position/color/radius/intensity where available.
2. Reconstruct sun direction, ambient light, fog and exposure through validated
   time-of-day keyframes; provide original/neutral lighting toggle.
3. Add local point/spot light transforms with physically plausible falloff
   and shadow support, then env-probe/cubemap assets.
4. Check in-game reference screenshots at identical camera/time-of-day
   settings. Original CryEngine deferred rendering, GI and baked probes
   cannot be assumed identical to wgpu output.

CryEngine level-file documentation:
https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1638401/pages/1605643

## Tutorial terrain and vegetation follow-up

The Tutorial map's activity log verifies 271/271 *requested scene asset
references*, but that number covers geometry/MTL reference verification, not
all DDS dependencies in those MTLs. In particular the authored source
`objects/man_made/vehicles/allterraincar01/allterraincar01.mtl` requests
`allterraincar01.tif` and `allterraincar01_spec.tif`; both must resolve
to exact owner-qualified cooked `.dds.0` or `.dds` in the same imported build.
When neither exists, the viewer now reports the first source-qualified
candidates searched, selected build, and the need to import the original
matching texture PAKs. The loader does NOT invent the missing textures.

The cross-PAK terrain resolver now checks
`Levels/<map>/LevelData/TerrainTexture.xml` before the old flattened
`Levels/<map>/TerrainTexture.xml`. The tile package remains
`Levels/<map>/terraintexture.pak`. Both are checked against the exact
map owner and unique build; if a source file truly is absent, the terrain
must remain neutral instead of borrowing another map's baked colors.
A synthetic multi-PAK Tutorial fixture verifies map-owned metadata and
tile-package references.

Vegetation materials preserve the original per-leaf `AlphaTest` threshold
from MTL, supporting the normalized 0..1 and editor percentage 0..100
encodings. A missing original value uses the documented 0.5 default.
The Level Editor sends each threshold to its world shader via a per-surface
uniform; the Model Viewer uses the previously unused fourth component of its
emission uniform, including after glow-slider updates. Only source-authored
Vegetation shader materials at full opacity enable diffuse-alpha cutouts;
do not apply packed diffuse alpha to opaque character body materials.

Windows acceptance remains necessary to confirm the source texture archives
are present and the rendered foliage matches Evolve at close-up zoom.
