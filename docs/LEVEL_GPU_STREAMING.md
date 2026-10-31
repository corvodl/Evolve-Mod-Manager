# Level Editor GPU streaming and source-color diagnostics (PR #158)

This is a progressive-upload renderer with **persistent per-scene GPU
residency**, not a claim that source textures, native CryEngine materials, or
Evolve visual parity are complete.
The source PAKs, imported files, level XML and full-resolution decoded CPU images
remain read-only/unchanged.

## Implemented

- The world renderer preflights visible groups before creating buffers or GPU
  textures. Groups nearest the camera are prioritized, with original terrain
  first. At most two new groups are uploaded per redraw; an on-demand UI timer
  requests more redraws while visible uploads remain.
- **Full quality** means original source dimensions (subject to GPU-supported
  texture size) and no manager-imposed 512 MiB scene-residency target.
  Already-uploaded groups stay GPU-resident while the scene stays loaded;
  camera movement does not evict them. Actual VRAM constraints can still
  trigger device loss or OOM when a scene exceeds the GPU's capacity.
  **Balanced** retains an estimated 384 MiB target and defers *new* assets
  rather than deleting ones already loaded.
- Identical decoded RGBA images are deduplicated across independent Arc
  allocations using a SHA-256 of dimensions + pixel bytes. The hash is
  precomputed in the existing map-import worker, not during GPU redraw.
  Roles and ground sampler class are part of GPU cache identity. Cache
  lifetime is one World, not a globally persistent untrusted image reference.
- Once GPU geometry and textures load, they remain resident across view
  changes. Frustum culling only skips drawing instances outside the view; it
  does not dispose of meshes or textures. Returning to a previously viewed
  area does not require a second upload. Scene unloading and explicit quality
  or scene replacement still release old GPU objects.
- **Scene Settings** reports resident/queued/deferred group counts, current
  texture-mip and geometry byte estimates, and peak estimates. The bounded
  crash log records `level_gpu_residency` periodically. These are estimates,
  not dedicated VRAM counters. Free physical VRAM cannot be obtained from the
  current wgpu Device interface; the UI explicitly reports it as unavailable.
- The map import activity includes sampled original decoded baked-terrain RGB
  values **before** sRGB sampling or original TimeOfDay lighting, including
  representative TL/TR/BL/BR atlas quadrants. The samples allow a tester to
  distinguish wrong source bytes and tile orientation from GPU color-space
  conversion or lighting. They do not establish Evolve in-game parity.

## Not yet implemented or verified

- Diffuse textures stream their mip images independently. To avoid a flat
  1×1 average color covering newly loaded objects, each source initializes a
  useful **up-to-128px mip** plus all coarser levels immediately; the shader's
  LOD floor prevents sampling uninitialized larger mips. Subsequent uploads
  rotate fairly across resident textures, writing at most **8 MiB** of rows
  per redraw. Original full-resolution mips remain available as streaming
  progresses.
  Auxiliary normal/specular/gloss/emissive/opacity maps still upload all their
  mips during GPU group admission; **full all-role independent mip streaming
  remains outstanding**. A CPU mip resize can still pause a frame, so an
  asynchronous decode worker or GPU mip-generation path remains desirable.
- Driver-provided actual remaining/dedicated memory and GPU allocation timing,
  including other applications and render targets.
- Oversized **multi-material** groups are now partitioned into independent
  GPU material-residency batches, reusing the same CPU mesh and authored
  placements. This may duplicate some GPU geometry but avoids deferring the
  entire mesh when several independent materials exceed the target together.
  Full mode no longer rejects otherwise valid materials solely because they
  exceed a 512 MiB *software* target; hardware limits still apply.
  Balanced can defer oversized groups. Sparse/tiled GPU textures and reliable
  driver free-VRAM checks remain outstanding.
- Exact Evolve baked terrain color, sRGB/linear conversion, layer blending,
  vegetation transmittance and engine HDR. No guessed color swap or tint has
  been added. Tutorial's four verified 1024² RGB24 tiles supply only a
  2048² source atlas; a higher-resolution ground texture cannot be invented.
- Material and geometry lookup now scopes both MTL/DDS and CGF dependencies
  by the selected canonical Game installation, even when legacy build labels
  overlap. Duplicate exact-path geometry is only accepted when actual file
  contents match by SHA-256. Local crash logs list every unresolved source
  model/material pair and reason for diagnosis; this does **not** fabricate
  missing original assets.
- The 22 model/material pairs without verified diffuse, 16 missing/unsupported
  geometry dependencies and ambiguous cross-PAK carcass CGFs reported by
  Tutorial. Streaming does **not** prove these missing source dependencies
  exist; don't resolve them by unverified basename matches.

## Real-map acceptance requirements

1. Open Tutorial and confirm the geometry/terrain resolve from the selected
   build only, with no cross-build asset mixing.
2. Observe `level_gpu_residency` and the Scene Settings resident/peak counters.
   Turn away from a loaded building, foliage patch and terrain, then turn back.
   All remain resident and should reappear immediately without GPU re-upload.
   Moving around must never decrease resident-group counts for the same scene
   and texture quality, unless a GPU/device error forces recovery.
3. Open Ice_Crashsite and a forest map. Check map responsiveness, deferred
   groups and failure handling with 4K source assets. A persistent deferred
   group is not considered successful full-quality coverage.
4. Compare baked atlas raw sampled RGB values against the imported tile bytes,
   an unlit atlas view and matched Evolve in-game screenshots at the same
   TimeOfDay; verify UV quadrant order and sRGB conversion before making a
   gameplay-color parity claim.
5. Check that source images are not rewritten and that switching builds or
   closing a world drops prior GPU handles. Test on GPUs with low and high
   memory independently; a green Rust CI check alone cannot establish this.

CryEngine 3.5.8 FreeSDK is a **general reference**, not proof of Evolve engine
behavior. No proprietary SDK or game data is part of this change.
