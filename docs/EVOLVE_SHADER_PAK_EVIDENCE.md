# Original Evolve shader archive evidence (2026-10-31)

## Provenance and extraction

The user supplied four original signed CryPAKs: `shaders.pak`, `shadersbin.pak`, `shadercache.pak`, and `engine.pak`. They were verified using a previously user-supplied 1024-bit original **public** RSA verification key via the project's documented RSA-PSS/Twofish signed CryPAK format. Extraction is local/read-only. **No keys, extracted shader source, engine configurations, game binaries, or PAK content are committed to this repository or packaged with the manager.**

| Source | Verified members | What it actually contains |
| --- | ---: | --- |
| `shaders.pak` | 36 | Readable `Shaders/*.ext` effect-generator property declarations; not full HLSL `.cfx` source |
| `shadersbin.pak` | 126 | `D3D11/*.cfxb` shader metadata and `.cfib` compiled include metadata (e.g., `Particles`, `Illum`, `SkyHDR`, `Terrain`) |
| `shadercache.pak` | 869 | `D3D11/*.fxcb` compiled shader variants, notably `Particles@ParticleDS`, `Illum` glow passes and deferred shading |
| `engine.pak` | 824 | Engine default materials/textures plus `Config/CVarGroups/*.cfg` rendering quality and postprocessing settings |

The .ext feature masks are **shader-specific**. A bit from the `Particles` generator does not mean the same thing as the identical bit in `Illum`, `Terrain` or `TRSBeam`. The `.cfxb`/`.fxcb` D3D11 binaries cannot be passed to wgpu as native WGSL or assumed to expose original CryEngine particle simulation source.

## Rendering behavior corroborated by source

- `Particles.ext`: `%MASK_TEXTURE` is hexadecimal **0x200** and explicitly multiplies the `Decal` texture into the particle. Other shader-local features include `%REFRACTION`, `%CAMERA_FADE`, `%DIFFUSE_ANIM`, screen-space/per-particle deformation, and `%ALPHAGLOW_CUSTOM` (custom red channel). The source MTLs often store these as **hex `GenMask`** with an empty `StringGenMask`; parsing only `StringGenMask` loses the feature selection. The Model Viewer and Level Editor's **static effect-CGF preview** now decode an exact same-build authored `Decal` DDS only if the `Particles` `GenMask` enables 0x200. A missing mask is diagnostic rather than a guessed white sprite. The shader-native emitter simulator, animation, distortion and soft particles remain separate work.
- `TRSBeam.ext`: `%GLOSS_DIFFUSEALPHA` 0x20 and `%ALPHAGLOW` 0x2000 mean diffuse alpha contains specular/glow control, **not mesh translucency**. The static viewer no longer treats those bits as an opacity instruction. `%BEAM_GAME_DRIVEN` and `%BEAM_STATIC_LENGTH` also show why an opaque or incorrectly placed beam tube is not a native beam.
- `Illum.ext`: supports `%ALPHAGLOW`, diffuse/gloss/detail maps and `%BLENDLAYER`; the material parser and viewer must preserve the authored channel roles, not infer opacity from arbitrary DDS alpha.
- `Terrain.ext`: confirms normal/gloss/detail variants exist; the baked terrain RGB atlas plus signed high-pass diffuse is still only a preview until native material-layer blending and projection are validated.
- `SkyHDR.ext`: confirms a specialized dynamic sky shader, not a static Sky material. Existing PR #165/168 analytical scattering remains **not** source-equivalent.
- `LightBeam.ext`/`GeometryBeam.ext`: dedicated noise, falloff, shadow interaction and UV-edge fading. Rendering a beam CGF with a stock opaque material is incorrect; PR #168 suppresses these unimplemented effect surfaces but does not provide original volumetric beams.

## High quality defaults (source `engine.pak`)

Selected original `Config/CVarGroups` default profile values: `r_TexturesStreamingSkipMips=0`, `r_HDRRendering=1`, `r_HDRBloomRatio=0.15`, `r_Beams=5`, `r_BeamsMaxSlices=200`, `r_DetailDistance=8`, `e_ViewDistRatioDetail=100`, and `e_ParticlesMinDrawPixels=1`. **These are CryEngine CVars, not drop-in world-unit distances or wgpu shader constants.** Do not copy their numeric values into an incompatible preview equation or silently change the user's texture budget. Their purpose here is to confirm the game's original capabilities and acceptance targets.

## Verification / follow-on acceptance

1. Import the four exact original PAKs into the Asset Library from the selected installation/build. The Level Editor's source inventory should show 36 `.ext`, 126 `.cfxb/.cfib`, 739 `.fxcb`, and original engine configuration entries. Only identical verified build sources can be combined.
2. Inspect a real `Particles` effect CGF whose MTL uses `GenMask=200` and an authored `Decal` mask (for example, a Goliath flame-breath material). Confirm the static alpha mask is visible in the Model Viewer and Level Editor; verify no opaque tubes or duplicate-alpha squaring.
3. Compare original in-game capture and editor renders for the same effect. Native dynamic sprite simulation, particle lighting, soft depth intersections, distortion/refraction, light beams, volumetric particles, camera exposure, HDR bloom, terrain projection and fog remain unverified.

## CI and source-compatibility guardrails

The source PAKs were inspected using the previously uploaded original **RSA public key** only, not an executable decompilation or external network service. No key bytes, exact shader declarations, HLSL/DXBC bytes, particle-library source XML, or private/proprietary assets are committed or distributed. The parser and static particle-mask preview use synthetic tests that encode only feature names and numeric masks, not shipped game code. The user should load these effects from **their own** verified game installation through the Asset Library. A static masked CGF is not equivalent to native animated particles, a true game-driven TRSBeam, or the engine's HDR renderer.


## HDR + bloom rendering milestone

The previous Model Viewer/Level Editor rendered directly to Iced's 8-bit swapchain. The scene was clipped or tone-mapped per object, so physically bright emissive/scene values could not contribute to a true bloom stage. The shared wgpu compositor now:

1. Renders Model Viewer and Level Editor primitives (including map sky, terrain, materials and point-light previews) into **linear Rgba16Float** scene color with the existing Depth32Float buffer.
2. Extracts radiance above a soft luminance threshold into a separate **quarter-resolution Rgba16Float** target; performs two Gaussian blur passes, with no unnecessary texture readbacks, no CPU work per pixel, and resize-only GPU allocation.
3. Adds the blurred radiance to the original linear HDR scene, then applies one ACES-style filmic tone map when compositing the scissored viewport into Iced's original presentation format.
4. Preserves the **Source RGB (unlit)** diagnostic by bypassing the filmic curve and bloom during map-color inspection. Outside the viewport, Iced's UI is untouched; alpha is forced opaque on composite.
5. Leaves original source texture pixels, shader feature masks, source PAK provenance, material textures, depth and raw FBX exports unchanged. Emission remains linear and positive until the final compositing pass instead of being normalized into low dynamic range.

This is **functional shared HDR/bloom**, not Evolve's native eye-adaptation, exposure-history, lens flare or proprietary shader implementation. The original shader PAKs and the high quality engine settings remain verification references, not redistributed runtime assets. Runtime acceptance still requires screenshots of the same Evolve effects in both viewers, along with GPU validation on Windows. Subsequent tasks are source-matched refraction/soft particle geometry and original particle simulation.
