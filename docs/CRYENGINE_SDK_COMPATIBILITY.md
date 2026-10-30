# CryEngine 3.5.8 FreeSDK compatibility and implementation roadmap

This developer-only reference documents the **user-supplied, intact six-volume** `CRYENGINE_Build_PC_v3_5_8_2310_freesdk` development SDK. The archive is a general CryEngine SDK, **not** the modified Evolve Stage 2 development kit. The SDK is private and ordinary manager users **will not have access to it**. None of its proprietary binaries, original data, or engine source should be committed, shipped, silently downloaded, imported by users, or used as interchangeable Stage 2 runtime DLLs. The manager must be self-contained, using original Rust implementations and appropriately licensed components included in the public distribution. SDK executables are developer-side format reference and validation only.

## Verified SDK inventory (not hypothetical)

| SDK evidence | Manager integration | Completion boundary |
|---|---|---|
| `Bin64/rc/rc.exe`, `rc.ini`, `ResourceCompilerFBX.dll`, `ResourceCompilerPC.dll` | Private development reference for a new **built-in Rust** FBX-to-CGF compiler | Native FBX→CGF creation and Evolve Stage 2 load not yet implemented/verified |
| `ResourceCompilerImage.dll`, `ResourceCompilerXML.dll`, `CryXML.dll`, `cudart64_42_9.dll` | Reference for integrated PC DDS texture presets and CryXML conversion | Independent Rust DDS and CryXML paths exist; proprietary plugin outputs still require parity validation |
| `CryCommon/CGFContent.h`, `IChunkFile.h`, `ICryAnimation.h` | Model chunks, vertex/normals/material subsets, skeleton/controller structures | Header contracts are not complete engine implementations |
| `GameSDK/Animations.pak` (CAF/DBA), `CryAction/IAnimatedCharacter.h` | Native animation regression fixtures, exact-rig playback, additive/layered clips and FBX export | Character attachments, 2012 variants and private-manager 1:1 playback unverified |
| `GameSDK/Levels/Singleplayer/Forest` | Heightmap, VegetationMap, layers, terrain texture PAK, TimeOfDay.xml, original effect references | Scene rendering, original GPU shaders, source camera and engine-grade physics unverified |
| `Engine/Shaders.pak`, `ParticleParams.h` | Shader/material inventory and particle schema reference | Cannot execute original shaders in wgpu or claim simulated particles |
| `Tools/GFxExport/gfxexport.exe` | Reference for Scaleform SWF/GFX export and UI resources | Not verified compatible with Evolve's UI runtime; preserve FFDec/Ruffle paths |
| `Tools/CryMaxTools`, `CryMayaTools` | Source rig, geometry, LOD/material, normals/tangents and authoring workflow research | Old Max/Maya plug-ins need version-specific hosts |
| `Code/GameSDK/GameDll/PlayerMovementController.cpp`, `PlayerStateFly.cpp` | CryAction movement/camera/flying semantics research | Example GameSDK code/offsets cannot be used as Evolve actor offsets |
| FMOD tools and library sources | Sound event/authoring inspection | Game banks/event compatibility not validated |

## Feature-track acceptance gates

### 1. Animations and full playback
- Decode and verify native CAF/DBA variants against source-matching CHR/SKIN, controller IDs, joint hierarchy, rest/inverse bind, and vertex weights.
- Whole-character CDF and equipment/socket assembly, optional clips, idle loaded paused, adjustable speed/scrub/loop; blend masks, additive overlays, aim/IK where verifiably authored.
- Preserve exporter rig, original normals/tangents, material slot ownership, UVs, and animation-folder Actions. Validate Stage 2, Legacy and 2012 test characters and the private-manager behavior.
- Do not call a clip playable solely because an archive lists it. Verify weighted vertex motion at multiple timestamps on the correct source rig.
- See `docs/ANIMATION_PLAYBACK_STATUS.md`.

### 2. Models, materials and texture tools
- Inspect original CGF/CHR/SKIN chunk versions, optional geometry companions, MTL subsets, DDS normals/spec/gloss/emissive/alpha and texture presets; preserve exact selected-build provenance.
- **No user SDK setup, no external compiler import and no private SDK downloads.** Implement supported conversion formats in the manager's Rust code. Initial built-in standalone PNG→PC DDS compiler now supports BC1/DXT1, BC3/DXT5 and BC5/ATI2 with complete mipchains, output validation and no overwrite. Existing manifest-checked PNG→DDS replacements and binary CryXmlB edit/encode remain supported.
- Native FBX→CGF, CHR/SKIN/CAF compilation and full RC texture preset parity are **not** implemented yet. Validate each independently against SDK-produced references privately, then Evolve Stage 2 test-game assets.
- Validate converted CGF in the Rust viewer first, then verify Stage 2 test-game load before offering an import/build action.

### 3. Level viewer/editor
- Render full original terrain atlas/vegetation placements/materials in one GPU scene, with reliable picking, transforms, undo/redo, source preservation, and project-copy save.
- Support authored TimeOfDay/Environment XML, original fog/HDR/sky and dynamic lights where wgpu equivalents are implemented. Original nonlinear spline flags/tangent curves require separate specification and tests.
- Support source particle emitters and shaders only as inventory until implemented GPU equivalents pass visual comparisons.
- The FreeSDK Forest TOD sample includes *out-of-day editor spline keys*. The PR importer now ignores those keys and reports the count, rather than discarding the whole original map curve or clamping outliers onto midnight.

### 4. XML, GFX, audio and other authored resources
- Browse/edit original or CryXML-converted material, particle, CDF, animation-set, level and environment XML in project copies with schema-aware validation and exact-source rebuild.
- Preserve GFX/Scaleform hover states, text/layout, external images and menu video; investigate `GFxExport` options separately from FFDec and Ruffle.
- Surface FMOD event references and audio metadata with matching source; conversion requires separate format tests.
- Developer interfaces are *reference material*; do not assume SDK ABI, signatures, offsets, game runtime APIs or fly-mode code work in Evolve Stage 2.

## Status and testing discipline

- **Implemented on this PR**: source-backed time-of-day partial preview; FX/shader inventory; bounded terrain/material recovery; tolerant outlier handling with diagnostics.
- **Elsewhere in existing manager**: basic native CAF and converted GLB skinned playback, additive/layered preview, partial Level Editor, GFX and audio integrations.
- **Not implemented or validated**: complete CDF/DBA character parity, high-performance full-world renderer, engine-equivalent particle/shader execution, full native equivalent of RC model/animation compilers, full GFX authoring/export parity, Evolve-specific engine runtime ABI.
- Public release policy: no user needs, downloads, installs, or is asked to locate this private CryEngine SDK. The native feature set ships in the manager itself. The separately maintained GPLv2 Cryengine-Converter remains a distinct optional tool subject to its own license and is **not** the private CryEngine SDK.
- CI: format, core/desktop tests, Clippy, Windows compilation and credential safeguards. Real Stage 2 and original SDK asset tests on Windows are mandatory before publishing claims of parity. Keep proprietary SDK files external and the PR draft until acceptance.
