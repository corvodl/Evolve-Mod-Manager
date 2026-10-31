# Level Editor: map-owned sky, atmosphere, lights and HDR (experimental)

This work extends draft PR #165. It uses the CryEngine 3.5.8-era FreeSDK **as an implementation reference only**. Original Evolve runtime shaders, sky cubemap layouts, and photometric constants must be checked against matching game builds before calling it native parity. No CryEngine SDK/game binary or authored asset is bundled.

## Source ownership and loading

- `LevelData/Environment.xml` provides a selected map's `SkyBox Material`, angle/stretching and its fog envelope. It is resolved by exact selected map and original imported Game-root provenance.
- `LevelData/TimeOfDay.xml` supplies keyed Sun/Sky/Fog/HDR curves. The environment renderer samples the selected map's curves at the editor hour.
- Original `Sky` material MTL references resolve in imported same-install PAKs. Three required `_12`, `_34`, `_5` DDS images are reconstructed from complete streamed mip fragments and uploaded to the GPU. Missing/ambiguous sources are diagnosed; a `SkyHDR` material does not claim an image-backed static sky.
- Original map placement-layer XML supplies source light class and nested properties, while `LevelScene` provides matching GUID-qualified world transforms, including hierarchy. Light placements missing authored **radius, color or intensity** are not invented.

## Rendering added

1. Opaque camera-centered background sky pass behind depth-tested terrain and objects. Static sky follows source angle and source sky-material textures; dynamic sky samples per-map Sun/Sky/Day/Night attributes and uses Rayleigh wavelength dependence and Henyey–Greenstein aerosol scattering.
2. Up to 24 camera-nearby authored local lights in the existing world shader, with attenuated diffuse/specular and authored projector half-angle cones when available. Selection uses actual radius, not generic billboard proxies. The light inventory itself is not limited to 24 until the bounded source parser cap.
3. Scene-key / min-max exposure, white point and saturation are sampled from original TimeOfDay curves. World and sky shaders perform linear-radiance highlight calculations and bounded filmic output mapping. Raw source terrain RGB mode remains unlit and uncorrected for diagnostics.
4. Shader-parser and synthetic source XML tests avoid distributing source assets.

## Known gaps before Evolve native parity

- **Atmosphere:** the SDK reference establishes physical Rayleigh/Mie roles, but the sky shader uses a single-scattering analytical **preview**, not the original Evolve optical-depth integrator, multiple scattering, clouds, shafts, stars, sun shadow interactions or verified wavelength/phase scaling.
- **Sky orientation:** the FreeSDK documents four side faces arranged in `_12` and `_34` and a zenith `_5`. Face orientation/stretch and source UVs require screenshot comparisons against the same build.
- **Lights:** point/spot diffuse/specular are implemented, but dynamic/cached shadows, LightBeam volumetric projector materials, projector cookies, clustered light lists, probes, area lights and physically calibrated Evolve falloff remain unimplemented. Some entity property spellings and rotations require source-fixture verification.
- **HDR:** exposure values and filmic tonemapping are source-driven, but the current Iced world renderer writes into the window surface. A 16-bit floating-point scene target, luminance histogram and temporal eye adaptation, volumetric/bloom HDR compositing, and the exact Evolve film curve are **not yet implemented**. Do not call the output native HDR parity.
- **Builds:** selected PAK Game root/known build labels must match; no assets from another build are substituted by matching basename. Missing Sky source does not trigger a fabricated sky-texture asset.
- **Validation:** run a Windows nightly acceptance on two different Stage 2 maps, exercise 0–24h sky and fog, compare screenshots to game at controlled exposure, verify local light/spot falloff and static sky horizon orientation, and check no GPU/CPU regressions or magenta/flickering textures.

## References

- CryEngine 3 *Setting Up Time of Day*: https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1114113/pages/1048857
- CryEngine 3 *Skybox Texture Setup*: https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1114113/pages/1310757
- CryEngine 3 *Environment Panel*: https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1114113/pages/11239516
- CryEngine 3 *Light Emitters*: https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1638401/pages/1605701
