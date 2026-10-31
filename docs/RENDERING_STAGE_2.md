# Rendering stage 2: source-backed effects (draft)

Work follows the merged PR #168/#169 nightly snapshot. No proprietary game PAK, particle XML or shader binaries are committed.

## Particle simulation

- Source data: original libs.pak contains packed CryXmlB ParticleLibrary declarations. Verified example map_ice_crashsite.xml has Rescue_Ship.RS_Lift_Jet.Jet_Base with Count=25, Continuous=true, ParticleLifeTime=0.4, Speed=-30, Size=4, and a real smoke texture reference.
- New evolve_core::particle_preview parses nested authored effects, actual Texture/Material, Count, ParticleLifeTime, Speed, Size, Alpha, Color, Continuous, Enabled and gravity. Deterministic fixed-substep simulation caps emitter instances (1024) and live particles (32768).
- Preview assumptions: Count per lifetime for continuous emission, velocity aligned to supplied emitter direction. Original CryEngine random envelopes, particle-age curves, pulse events, GPU collision/physics, spawning geometry, atlas frame animation and nested runtime event behavior are NOT yet native parity.
- Simulation core is not yet wired to original map particle placements or the GPU sprite renderer. The code provides a tested, bounded basis for the subsequent integration, not a claim that smoke/fire has appeared in the editor.

## Depth-tested spotlight scattering

- The Level Editor now has a depth-tested additive camera-facing spotlight sheet from actual map Light sources, original position, direction, radius and cone, bound to at most 64 directional spotlights. No omnidirectional point light is converted to a cone. It runs in the HDR viewport ahead of bloom.
- This preview is a soft cone billboard rather than full raymarching. Original noise maps, projector textures, temporal scattering, local fog volumes and shadow interactions remain unimplemented. Do not claim native volumetric parity.

## Scene-color refraction foundation

- The viewport_refraction.wgsl shader contains depth-aware scene-color displacement controlled by explicit normal and coverage textures, and masks offsets across nearby depth silhouettes. Its separate input scene-color snapshot avoids a GPU read/write hazard.
- The shader is not yet bound to live material geometry. It must remain source-gated until exact Glass/Water/Particle REFRACTION material flags and normal/coverage maps are verified. Applying it indiscriminately to opaque Illum or beam geometry would damage correct rendering.

## Acceptance

- Verify cargo fmt, core and Windows desktop tests/Clippy, and WGSL parser tests. Synthetic tests only, never original assets.
- Visual-test source spotlights and HDR on Ice_Crashsite, source texture colors, geometry depth and render order. Validate particle emitter names against same-build imported libs.pak before rendering.
- Next: source-linked emitter placement, texture atlases, GPU sprite instancing and effects-browser playback; refraction coverage mask and scene-color capture; native raymarched fog/beam lighting.
