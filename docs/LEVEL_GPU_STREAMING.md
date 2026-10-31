# Level Editor GPU streaming and source-color diagnostics (PR #158)

This is a **first bounded-residency implementation**, not a claim that source
textures, native CryEngine material appearance, or Evolve GPU parity are complete.
The source PAKs, imported files, level XML and full-resolution decoded CPU images
remain read-only/unchanged.

## Implemented

- The world renderer preflights visible groups before creating buffers or GPU
  textures. Groups nearest the camera are prioritized, with original terrain
  first. At most two new groups are uploaded per redraw; an on-demand UI timer
  requests more redraws while visible uploads remain.
- **Full quality** means original source dimensions (subject to the GPU's
  maximum supported texture size), rather than an arbitrary 2K/4K cap.
  A conservative **512 MiB estimated world-residency target** constrains
  simultaneous material + geometry allocations, not the original source data.
  If a group cannot fit, it is deferred instead of risking a wgpu OOM panic.
  Moving the camera can free offscreen GPU handles and retry its source files.
- Identical decoded RGBA images are deduplicated across independent Arc
  allocations using a SHA-256 of dimensions + pixel bytes. Roles and ground
  sampler class are part of GPU cache identity. Cache lifetime is one World,
  not a globally persistent untrusted image reference.
- Shader/renderer source images retain full resolution on the CPU while GPU
  cache resources for offscreen groups are released. Visible nearer groups may
  replace farther resident groups only if closer, avoiding ping-pong residency.
- **Scene Settings** reports resident/queued/deferred group counts, current
  texture-mip and geometry byte estimates, and peak estimates. The bounded
  crash log records `level_gpu_residency` periodically. These are estimates,
  not dedicated VRAM counters. Free physical VRAM cannot be obtained from the
  current wgpu Device interface; the UI explicitly reports it as unavailable.
- The map import activity includes sampled original decoded baked-terrain RGB
  values **before** sRGB sampling or original TimeOfDay lighting. The sample
  allows a tester to distinguish wrong raw tile bytes from wrong GPU coloring.

## Not yet implemented or verified

- Per-mipmap and per-texture *intra-group* asynchronous uploads. Each individual
  admitted material group still creates and fills its full mip chain together;
  unusually large groups can stall for a frame. The current pacing is by mesh
  group, not by bytes or elapsed GPU time.
- Driver-provided actual remaining/dedicated memory and GPU allocation timing,
  including other applications and render targets.
- Fully GPU-resident fallback for a visible group larger than the 512 MiB
  residency target. Such groups are **reported as deferred**, rather than
  downsampled and falsely described as full-quality.
- Exact Evolve baked terrain color, sRGB/linear conversion, layer blending,
  vegetation transmittance and engine HDR. No guessed color swap or tint has
  been added. Tutorial's four verified 1024² RGB24 tiles supply only a
  2048² source atlas; a higher-resolution ground texture cannot be invented.
- The 22 model/material pairs without verified diffuse, 16 missing/unsupported
  geometry dependencies and ambiguous cross-PAK carcass CGFs reported by
  Tutorial. Streaming does **not** prove these missing source dependencies
  exist; don't resolve them by unverified basename matches.

## Real-map acceptance requirements

1. Open Tutorial and confirm the geometry/terrain resolve from the selected
   build only, with no cross-build asset mixing.
2. Observe `level_gpu_residency` and the Scene Settings resident/peak counters.
   Turning the camera away should release offscreen residency; returning should
   reload the exact original source images, not fabricated neutral maps.
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
