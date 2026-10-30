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

## Scoped follow-ups from read-only inspection (October 30, 2026)

Basis: everything below was read from `CRYENGINE_Build_PC_v3_5_8_2310_freesdk`. Archive entries were enumerated and read in memory by a developer-side script; nothing was extracted into the repository, no SDK executable was run, and no proprietary asset is redistributed. Counts are for this exact build only. The manager must recompute them from the user-selected build and must never substitute FreeSDK values for Evolve Stage 2 data.

### A. `.animevents` timeline markers (animation viewer)

**SDK evidence**

- `Code/CryEngine/CryCommon/ICryAnimation.h:1090-1105` defines `IAnimEvents::GetAnimEventList(const char* animationFilePath)`, `SaveAnimEventToXml`/`LoadAnimEventFromXml` and `InitializeSegmentationDataFromAnimEvents`: the per-animation XML event list is the authored timeline, and segmentation data is derived from it.
- `Code/CryEngine/CryCommon/CryCharAnimationParams.h:245-324` defines `CAnimEventData`: normalized time/end time, `SetNormalizedTime` clamps to 0..1 and raises the end time to at least the start (262-270), name plus lowercase CRC32 (272-279), custom parameter (281-282), bone (284-285), model (287-288), offset/direction (290-294). `AnimEventInstance` (328-374) exposes the same fields at playback time.
- `Animations.pak` contains exactly two: `Animations/human/male/events.animevents` (915,158 B) and `Animations/weapons/rifle/rifle.animevents` (1,033 B). `Objects.pak` adds `Objects/weapons/ammo/claymore/claymore_eventdatabase.animevents` (2,433 B) and `Objects/weapons/equipment/medical_kit/medical_kit_eventdatabase.animevents` (1,377 B).
- Schema: `<anim_event_list><animation name="…caf"><event name time [endTime] parameter bone offset dir model/></animation>`. `rifle.animevents` omits `endTime`; all 5,131 events in the human file include it (frequently equal to `time`).
- Human file: 1,550 animation entries / 5,131 events / 124 distinct event names. Most frequent: `sound` 1,858, `footstep` 1,212, `foley` 1,016, `ForceFeedback` 438, `Segment1` 76, `ForbidReactions` 70, `DeathReactionEnd` 64, `RagdollStart` 63, `Segment2` 37, `rightHand` 31, `stow` 21, `effect` 20. 19 distinct non-empty `bone` values, one non-empty `model` value, no non-zero `offset` or `dir`.
- This branch now parses the declared `.animevents` file in `crates/evolve-core/src/animation_events.rs`, resolves it from the same-build character `.chrparams`, and shows read-only, clickable event timestamps in the Animation Browser. This is marker inspection, not execution of the authored effects. Clip duration originates in `crates/evolve-core/src/native_caf.rs`; the timeline UI is in `crates/evolve-desktop/src/asset_viewer.rs`.

**Scope (read-only)**

- Resolve sidecars from the selected-build inventory with the same strict path rules as the CAF, then match `animation/@name` to the loaded clip by normalized path (case-insensitive, forward slashes).
- Place markers at `time × clip.duration_seconds()`; when `endTime` is present and later than `time`, render a range instead of a point.
- Show name, parameter, bone and model in a read-only tooltip; group unresolved clips/sidecars into import diagnostics.
- Do not trigger the referenced sound, particle, ragdoll, force-feedback or flow-graph events, and never edit or reserialize the sidecar.

**Acceptance tests**

1. Parse both attribute shapes from inline fixtures: rifle-style (no `endTime`) yields `end == start`; human-style preserves the explicit end.
2. Clamping matches `CryCharAnimationParams.h:262-270`: `<0`→0, `>1`→1, `endTime < time`→`time`, each with a diagnostic rather than silent repair.
3. Path matching is case-insensitive with slash normalization; a basename-only match is rejected and reported.
4. Duplicate `(name, time)` events keep source order and all markers.
5. Marker position equals `time × duration` within 1e-4 and does not move under Play/Pause/scrub or CAF layer-weight changes.
6. Oversized file/event/string or malformed XML fails closed to an empty timeline plus a reason; no panic and no writes.
7. A clip with no sidecar reports "no authored events": the rifle sidecar covers only 4 animations, so per-clip absence is normal and must not be treated as an error.

**Evolve Stage 2 validation**

- Read-only scan of the user's selected Stage 2 build for `.animevents`; show the exact archive path and entry count used, or "none found".
- A read-only spot check of the user's locally imported `characters_creatures_armadonbull_data.pak` found `characters/creatures/armadonbull/armadonbull.animevents` with full virtual CAF paths and `foley`/`footstep` events. The optional local parser test found 58 timelines, 214 events and 0 diagnostics without modifying the source. This is a real Evolve-format example, not a validated UI marker or proof that every build contains matching sidecars.
- Open one Stage 2 character CAF and one weapon reload; screenshot markers at 0/mid/1 and after scrubbing; verify source hashes are unchanged before/after.
- Never borrow FreeSDK event names for Evolve clips. Unresolved names stay listed as unresolved.

### B. Surface-type → material-effect relationship inspector

**SDK evidence**

- `Code/CryEngine/CryCommon/ISurfaceType.h:24-31` defines the flags (`NO_PHYSICALIZE`, `NO_COLLIDE` for canopy only, `VEHICLE_ONLY_COLLISION`, `CAN_SHATTER`, `BULLET_PIERCEABLE` — "used by MFX system to spawn front/back FX"). `68-124` defines `SPhysicalParams`, `SBreakable2DParams` and `SBreakageParticles`; `132-170` exposes breakability, hit points and breakage particles by type; `193-210` is the name→type manager.
- `Code/CryEngine/CryCommon/IMaterialEffects.h:545-574` defines lookup by `(lib, effect)`, `(surface1, surface2)`, `(customName, surface)` and `(entityClass, surface)`, plus `GetResources`, `ExecuteEffect`, `GetDefaultSurfaceIndex`, `GetDefaultCanopyIndex` and `PlayBreakageEffect(surface, breakageType, …)`. `28-37` defines the `MFX_PLAY_*` resource kinds.
- `GameData.pak/Libs/MaterialEffects/SurfaceTypes.xml` has 48 `SurfaceType` entries, 45 `Physics`, 33 `BreakageParticles`, 16 `AI` and 19 entries with a `type=` grouping (e.g. `mat_glass` `type="glass"`). It uses a lower-case `<breakable_2d>` element and `Physics` attributes such as `hit_maxhits` that do not exist in `SPhysicalParams` (`ISurfaceType.h:68-88`), so unknown attributes must be preserved, not rejected.
- `GameData.pak/Libs/MaterialEffects/MaterialEffects.xml` (279,647 B) is an Excel 2003 spreadsheet XML (`urn:schemas-microsoft-com:office:spreadsheet`, `RefModeR1C1`), not a plain matrix: 44 named columns (surface types), 73 data rows = 44 surface-name rows + 25 custom-query rows (`PistolBullet`, `RifleBullet`, `ShotgunShell`, `footstep_player`, `Breakage:Breakage`, …) + 4 blank rows. A direct XML inventory found 2,949 nonempty data cells; an earlier normalized `lib:effect` count found 2,829. These are different measures and the 120-cell difference needs classification before a parity assertion. `ss:Index` skips cells; an empty `<Cell/>` means "no effect", not "default".
- Four surfaces have neither a matrix row nor a column: `mat_glass_breakable_thin_small`, `mat_glass_breakable_thin_large`, `mat_glass_breakable_safetyglass_small`, `mat_glass_breakable_safetyglass_large`.
- 25 cells reference effects that exist in none of the 13 `FXLibs/*.xml`: `collisions:canopy_canopy` ×12, `foley:bf_mud` ×12, `collisions:small_shell_default` ×1.
- FXLib schema (`FXLibs/bulletimpacts.xml`): `<Effect name delay>` containing `<RandEffect>` groups of `<Particle><Name minscale maxscale maxscaledist>`, `<Decal minscale maxscale><Material>`, `<FlowGraph name maxdist>` and `<Sound><Name>`; 13 FXLib files and 5 Flowgraph files ship in this build.
- 353 of the 373 `Objects.pak` MTL files declare `SurfaceType="mat_…"`. `MaterialSlot` (`crates/evolve-core/src/materials.rs:77-102`) has no surface-type field, and `ResourceSummary` (`crates/evolve-desktop/src/level_environment.rs`) counts only particle XML, MTL and CFX files.

**Scope (read-only)**

- For a selected MTL/subset: `SurfaceType` → surface definition (flags / `Physics` / `BreakageParticles` / `AI`) → chosen matrix cell (or custom-name row) → effect → resource list → resolution status (present/dangling).
- Parse the workbook tolerantly (namespace, `ss:Index`, blank cells, duplicate row names, custom rows) and list unmodeled attributes so schema drift is visible.
- No effect execution, audio, decal placement or file mutation; `mat_default` is shown only when the engine's default-surface path is actually selected.

**Acceptance tests**

1. Fixture workbook with an `ss:Index="2"` header, trailing blank cells, one blank `<Cell/>` and a custom-name row: assert exact row/column mapping and that blank ≠ default.
2. Surface lookup is case-insensitive but does not fuzz names (`mat_metal_extra` ≠ `mat_metal`); unknown surfaces report "not defined" instead of silently falling back.
3. On the private FreeSDK, a developer-only report must reproduce 48 surfaces / 44 columns / 73 rows, classify the 2,949 nonempty cells and verify 4 surfaces without a row/column; CI uses only synthetic fixtures. Do not require 2,829 cells without resolving the differing count methods.
4. MTL without `SurfaceType` → explicit "unassigned"; MTL with an undefined name → dangling diagnostic; neither becomes `mat_default`.
5. `BreakageParticles` type lookup prefers the specific entry before falling back to the default (`GetBreakageParticles(type, lookInDefault=true)`, `ISurfaceType.h:163`).
6. Inspector mutation test: file bytes unchanged after open/navigate; no audio or particle callback is invoked.

**Evolve Stage 2 validation**

- Inspect the user's selected Stage 2 `Libs/MaterialEffects` plus one real Stage 2 MTL/subset; show that build's own names and counts.
- An isolated read-only source probe of an Evolve `build_3` `Libs/MaterialEffects` copy parsed 46 surfaces, 43 named columns, 178 rows, 7,298 recognized effect cells and 19 FXLib files. This is schema compatibility evidence, not validation of an in-game surface interaction or a complete manager inspector.
- Confirm an Evolve material's surface type reaches the correct row/cell and that a missing cell is reported as missing rather than substituted from the FreeSDK matrix.
- Inspection only: no effect plays, no source file changes.

### C. Time-of-day tangent interpolation

**SDK evidence**

- `Code/CryEngine/CryCommon/ISplines.h:24-52` defines `ESplineKeyTangentType` `NONE=0`, `CUSTOM=1`, `ZERO=2`, `STEP=3`, `LINEAR=4`; key flags store the in-tangent in bits 0-2, the out-tangent in bits 3-5, a unify bit at 6, while bits 16-19 are editor UI-selection bits (`ESPLINE_KEY_UI_SELECTED_MASK`). The serialized key is `time:value:flags`.
- `ISplines.h:661-712` computes slopes (LINEAR keys use the segment chord; auto keys use 0.5·(next − previous)); `786-856` applies STEP (hold), ZERO (flat) and LINEAR per side; `947-957` returns the neighbor value across a STEP boundary.
- `GameSDK/Levels/Singleplayer/Forest/LevelData/TimeOfDay.xml` has 107 variables / 1,669 keys / 12 out-of-range editor keys. Flag histogram: `0`×1,169, `36`×482, `65536`×7, `458752`×3, `65556`×2, and one each of `28`, `24`, `20`, `35`, `65572`, `3`. Four curves carry STEP/ZERO tangents: `Skybox multiplier` (`35` at 0, `3` at 1.0), `Volumetric fog: Shadow darkening sun` (`28`, `24`), `Sky color multiplier` (`65556` ×2), `Night sky: Moon inner corona scale` (`20`). A fifth, `Sky light: Rayleigh scattering` (`65572`), is LINEAR/LINEAR plus a UI-selection bit and must sample like flag `36`.
- `Editor/CryDesigner_TimeOfDay.xml` (editor-side sample) mixes selection bits with tangents (`65572`, `458788`) and uses one key per curve.
- The shipped `Code` tree contains only the `ITimeOfDay` interface (`I3DEngine.h:972-1206`); no TimeOfDay spline implementation is included, so the exact auto-smooth (`NONE`) evaluator cannot be confirmed from SDK source.
- Manager status: this branch now decodes the in/out tangent bits and evaluates a cubic Bezier basis with a linear fallback for unsupported modes (`Curve::sample` in `crates/evolve-desktop/src/level_environment.rs`, commit "Preview TimeOfDay splines with SDK tangent flags"); earlier builds sampled linearly. Diagnostics are emitted by `crates/evolve-desktop/src/level_library.rs:652-660` and `crates/evolve-desktop/src/level_workspace.rs:476-479`. The gates below decide when the preview may stop being labelled approximate.

**Scope (phased, no reserialization)**

1. Lock the flag decode: mask UI-selection bits 16-19 and never let editor state change a sample; count tangent types per curve in diagnostics.
2. Validate STEP/ZERO/LINEAR against the `ISplines.h` formulas; keep the unsupported-mode path linear and explicitly reported rather than presented as engine output.
3. Treat `CUSTOM` as unsupported: the `time:value:flags` field carries no tangent magnitude (and Forest has none), so diagnose instead of inventing handles.
4. Keep duplicate-time handling explicit (`keys()` sort + dedup in `level_environment.rs`); never emit NaN or zero-length segments.
5. Never rewrite the original XML, and never extrapolate beyond authored keys.

**Acceptance tests**

1. Bit-decode table: `0`→(NONE,NONE); `36`→(LINEAR,LINEAR); `20`→(LINEAR,ZERO); `28`→(LINEAR,STEP); `24`→(NONE,STEP); `35`→(STEP,LINEAR); `3`→(STEP,NONE); `65556`→(LINEAR,ZERO + one UI bit); `65572`→(LINEAR,LINEAR + one); `458752`→(NONE,NONE + three); `458788`→(LINEAR,LINEAR + three); unify bit `64` surfaced separately.
2. STEP boundary: with `0.75:1:36,1:0:3`, sampling at 23.0 h must be exactly 1.0 and at 24.0 h 0.0 (the previous linear preview returns ≈0.167 at 23.0 h).
3. ZERO tangent: at `0.5:0.76:65556` (the second `65556` key of `Sky color multiplier`) the right-hand derivative is ≈0 (finite-difference tolerance 1e-3), whereas the old linear preview leaves 0.76 toward 3.07738 at 0.5625 at ≈37 per normalized unit.
4. LINEAR: between two LINEAR keys, samples at 0.25/0.5/0.75 of the segment stay exactly on the chord.
5. Regression: Forest parses 107 variables / 1,669 keys and keeps skipping 12 out-of-range keys, with the 5 non-default-flag curves classified correctly (4 STEP/ZERO, 1 LINEAR + UI bit). `NONE`-tangent golden samples are intentionally different from the old linear preview and must be re-reviewed; `LINEAR`-key segments stay exactly on the chord.
6. UI-selection bits never change a sample: identical curves carrying `0`, `65536`, `458752` (and the `65572`/`458788` pair) must evaluate identically. Non-integer or missing flags fail parsing, and an unsupported tangent (`CUSTOM`) is reported as approximate while returning the documented linear fallback.

**Evolve Stage 2 validation**

- Parse the user's selected Stage 2 TOD files and print the same flag histogram and flagged-variable list.
- Screenshot an Evolve map immediately before/after each STEP/ZERO key and compare with the previously approved linear-preview screenshots; only then expose tangent-aware sampling.
- Keep `NONE` and `CUSTOM` explicitly marked approximate in the UI until an independently validated evaluator exists.

### Additional concrete findings

**D1. TOD root playback window (now read-only metadata).** `GameSDK/Levels/Singleplayer/Forest/LevelData/TimeOfDay.xml:1` carries `Time="9.75" TimeStart="9.75" TimeEnd="23.833334" TimeAnimSpeed="0"`, and `Editor/CryDesigner_TimeOfDay.xml:1` carries `Time="12" TimeStart="12" TimeEnd="12" TimeAnimSpeed="0"`. `TimeOfDay::parse` now validates and displays these three additional attributes (0–24, finite, start ≤ end), reporting malformed or incomplete windows while retaining the lighting profile. The manual 0–24h slider is deliberately unchanged; automatic authored playback is not implemented. Tests cover exact Forest values, missing fields and invalid/non-finite windows. Evolve: display the selected map's own authored window, screenshot at `TimeStart`/`TimeEnd`, and never substitute the FreeSDK window.

**D2. Per-CAF `.animsettings` sidecars are ignored.** `Animations.pak` contains 1,566 `.animsettings` entries (106–2,715 B); 358 set `AdditiveAnimation value="1"`. Schema: `<AnimationSettings><AdditiveAnimation value/><Skeleton value/><CompressionSettings><Version/><Compression/><RotEpsilon/><PosEpsilon/><PerBoneCompression><Bone Name Multiply/>…</PerBoneCompression></CompressionSettings></AnimationSettings>`, where `PerBoneCompression` carries per-bone multipliers (0.1/10) and accounts for the 2,715 B maximum. Examples: `Animations/human/male/additive/Additive_Breathing_heavy_01.animsettings` uses `skeleton_player_generic`; `Animations/animals/amphibians/toad/toad_idle01.animsettings` uses `toad_skel`. The manager never reads them (`git grep -i animsettings -- '*.rs'` is empty) and takes `additive` only from the CAF asset flags (`native_caf.rs:370`, surfaced by `main.rs:13508`; `caf_layers.rs:178`). Scope: read-only sidecar resolution next to the selected CAF plus a parity report (sidecar additive vs CAF additive, skeleton alias, compression block). Do not change playback or retargeting until parity is verified. Tests: stem matching (`.caf`↔`.animsettings`), missing sidecar is not an error, malformed sidecar → diagnostic, no cross-build pairing, and additive mismatch is shown without silently overriding the CAF flag. Evolve: scan the selected Stage 2 build, report counts, compare one known additive idle, and keep the CAF-derived state authoritative until parity is proven.

B has a bounded read-only source parser and a CLI surface-pair inspector, not physics/effect execution or a Level Editor material inspector. D2 is not implemented; A is read-only event marker inspection, not event execution; D1 is read-only metadata, not authored playback. C now has a flag-aware Bezier preview, but none of its native parity gates above has been met. The remaining features require synthetic unit fixtures plus the private-build and Evolve checks listed. None authorizes shipping SDK data or executing SDK binaries.

### Native level effects: completion boundaries

The present editor previews emissive source textures and MTL strengths, inventories particle definitions/placements, and approximates source TimeOfDay sun/sky curves. This is **not** complete CryEngine rendering. `Code/CryEngine/CryCommon/ParticleParams.h` and `IParticles.h` describe emitter parameters and interfaces, but provide neither an implementation of the particle simulation nor Evolve's game-specific effects. `I3DEngine.h` describes TimeOfDay interfaces, while `Engine/Shaders.pak` supplies proprietary shader assets rather than a distributable wgpu implementation. In particular, the existing `world_instances.wgsl` contains an intentionally approximate emissive tonemap and no native shadow/GI/fog pass. Do not silently call those native features supported just because authored data was detected.

Incremental delivery gates: (1) retain the authored emissive map, intensity, alpha and material-slot provenance with regression images, without guessing unsupported shader parameters; (2) parse and inspect source particle parameters and per-emitter references with bounded, same-build diagnostics before offering a clearly labelled simulated subset; (3) add independently tested sky/sun/fog/HDR/shadow components only with explicit source and approximation labels; (4) compare approved daylight, twilight, night, emission and moving-emitter screenshots against the user's original Evolve build. Engine-exact particles/shaders/GI require their own licensed, independently implemented renderer and cannot be inferred from this FreeSDK inventory. Original game assets and SDK binaries remain outside the repository and release artifacts.
