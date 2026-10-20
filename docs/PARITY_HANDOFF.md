# Public Rust manager parity — continuation notes

Updated: **2026-10-20** · Repository: `corvodl/Evolve-Mod-Manager` · Working PR: **#6** on `feature/blender-viewport-cdf-autoload`, based on PR #5's `feature/pr5-combined-viewer-preview`. This document is a resumable engineering checklist, **not** a claim of production parity. See [PARITY.md](PARITY.md) for the broader feature matrix.

## Resume here first

1. Inspect PR #6's latest commit, GitHub Actions **core**, **desktop** and **Windows portable preview** conclusions. Do not assume an older green CI result covers the newest commit.
2. Run `cargo fmt --all -- --check`, `cargo test -p evolve-core --all-targets`, `cargo clippy -p evolve-core --all-targets -- -D warnings`, `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace --all-targets`. Fix failing tests without disabling lint or source validation.
3. Resume **real Blender 5.2 character acceptance** for unified FBX, then native character CAF/DBA and offline installed-game workflows. Any real extracted assets remain out of Git and release packages.
4. Update this file after each verified advance with the exact commit/run, observation, blockers, and next operation. Keep PR open until representative real-asset acceptance and CI pass; no release just for green synthetic tests.

## October 20 continuation — 2012 equipment, verified rigging and Codex texture handoff

**Persisted in PR #6:** latest code HEAD before this documentation update was `17d1bd241ba2520d1c45a2f1fd7dacc650123f4a`. GitHub Actions [run 39469670052](https://github.com/corvodl/Evolve-Mod-Manager/actions/runs/39469670052) completed **successfully** for Linux core, Windows desktop, and Windows portable preview (not a release). Those results are for that exact code commit, **not** for an uncommitted Codex workspace or real extracted game assets.

- **New viewer sidebar Equipment tab:** `crates/evolve-desktop/src/asset_viewer.rs`, `main.rs` and `crates/evolve-core/src/character_equipment.rs` expose separate exact-path left/right choices for historical Scorpid arms. `read_static_cdf_assembly_with_equipment` validates each choice and uses the override for the preview without editing the original CDF. `export_unified_cdf_fbx_with_equipment` carries current choices to verified rigged FBX. Selections are scoped to the current CDF and reject unknown/cross-character variants. Only CDF-indexed alternative .chr families are currently offered; other equipment types need actual authored slot evidence.
- **Historical 2012 rigging:** `legacy_rig.rs` validates CryTek `0x745` original 584-byte bone records, parent hierarchy, inverse/rest matrices, stream 9 four-weight influences (exact 255 sum), and subset-local bone palettes. It remaps weighted bones to the master skeleton only if identity, ancestry and bind matrices agree; otherwise the original body-local bind pose wins. `rig.rs` now emits original-weight skinned GLB/FBX for these sources and refuses to mislabel historic DBA/CAF data as verified Stage 2 animation. Synthetic positive and rejection fixtures passed in the CI run above; Blender 5.2 reimport with real 2012 characters is **not yet verified**.
- **Scorpid default assembly:** the repository has a synthetic regression for five authored Scorpid parts (body/rig, head, petapalps, left Basher, right Basher) with unavailable bone sockets. This does **not** prove real Scorpid CDF/equipment preview is complete or fully deformable.

**Separate Codex session, stopped by usage limit (user-provided screenshot, not persisted or independently retested in PR #6):** The session reported that a previously corrupted 2012 console DDS sample decoded correctly after **Xbox 360 tiled block-address reconstruction plus 16-bit byte swapping**, and that the decoded bytes corresponded to the original PAK sample (extraction itself was not the corruption cause). It also reported discovering that **low/high mip fragments span separate imported PAKs**, while the decoder looked only beside the DDS header. Finally it reported getting Scorpid's two default Basher arms to load by tolerating empty socket declarations and mapping each arm's reduced bone table against the main skeleton. These are **reported observations**, not proof those final changes were saved, committed, or tested in this repository.

Current public Rust code check:
- `dds.rs` already has `xbox360_tiled_xy` / `untile_xbox360` and `decode_dds_rgba_with_layout(..., Xbox360Tiled)`, with a synthetic 128×128 block regression. The standard `decode_dds_rgba` still calls `Linear` unconditionally. **Missing:** verified header/platform-flag-triggered layout selection at all preview/export entrypoints. Do not untile ordinary linear DDS files.
- `load_dds` still enumerates numbered mip fragments via `fs::read_dir(parent)`. **Missing:** resolving each exact numbered virtual DDS fragment across imported archives of the *same registered build*, with duplicate/ambiguous and byte-size validation and no cross-build borrowing.
- Current CDF assembly and historical rig readers are more capable than the earlier iteration, but the **specific empty-socket and reduced-arm-table fixes reported by Codex are not independently linked to a Git commit**. Compare the Codex working tree/diff against `17d1bd2` before reimplementing or declaring these fixes done.

**Next worker tasks, in order:**
1. Recover the interrupted Codex workspace/diff if available; do not assume its uncommitted patches exist in Git. Reproduce the problem using the private sample outside the repository; record checksum, DDS header flags, source PAK IDs/build and original path privately (never commit copyrighted assets).
2. Implement strictly header-flag-controlled Xbox 360 untile + 16-bit swap in the common DDS decode call sites; test console vs standard linear DDS and unsupported layouts. Preserve raw extracted PAK bytes.
3. Resolve highest-quality mip fragments by exact case-insensitive virtual path from the same registered imported build, detect ambiguity, enforce expected mip byte lengths and include direct-vs-split DDS regressions.
4. Test **Scorpid 1/2/3** default CDF arm loads, all six independent left/right alternatives, socket declarations, reduced bone-table remapping, equipment switches and character reloads; check real GPU textured render, source bone IDs and CDF FBX reimport. Do not substitute guessed palette indices or fake limb weights.
5. Run formatter, core + Windows workspace tests, portable preview, and real 2012 sample smoke tests on the final commit. Update this section with exact new commit/run and blockers; **do not merge or release** from synthetic green CI alone.

## Viewer regression work — indexed .skin and CDF assembly, authored GPU textures

User validation on 2026-10-20 identified: standalone `.skin` selections and full CDF assemblies were not viewable, associated equipment did not attach, and an 11-slot MTL showed 23 resolved DDS references but no per-face GPU textures. The screenshot was intentionally in **wireframe mode** to demonstrate source geometry; red wireframe by itself is not a texture failure.

Implemented on PR #6 after last fully green run, **pending the latest CI and real-game acceptance**:

- `auto_resources::exact_render_companion`: selecting a native `.skin`/ `.chr`/ `.cgf`/ `.cga` tries only its same-virtual-name compiled `.skinm`/ `.chrm`/ `.cgfm`/ `.cgam` via indexed assets or the manifest-verified imported PAK library. It does not select a similar filename or combine archives/builds without provenance.
- `Operation::InspectIndexedModel` and `State.render_source`: preview the exact render companion while retaining the user-selected skeletal binding in the explorer. CAF binding and GPU surface loading follow the render source, including one stored in another imported PAK. Missing compiled companions must produce actionable errors rather than silently loading an unrelated mesh.
- `read_static_cdf_assembly_with_imported`: allow a CDF to draw original `.skinm` geometry when the separate `.skin` metadata or compiled skeleton is missing. The report explicitly warns that **original skin weights, bone sockets and animation cannot be verified**; missing/ambiguous socket transforms are never invented. A genuine `CA_BONE` still requires the original verified bind-pose skeleton and matching `BoneName`.
- `assembly_materials::load_model` and main background job: after MTL discovery, upload original per-subset diffuse/normal/emissive/specular/gloss maps using strict `0x08001017` indices. With multiple MTL leaves but no original source subset mapping, refuse to paint all triangles with slot zero. Preserve NoDraw/hole face spans. The inspector reports **actual GPU-bound original subsets**, not only resolved MTL asset counts.
- Viewer layout: explorer defaults to the **left** for new settings, with a one-click left/right toggle. Shaded/wireframe switching is visible in the viewport toolbar. Existing saved explorer placement preferences are preserved.
- Synthetic tests cover precise `.skinm` companion choice, CDF static fallback when an original skeleton and separate `.skin` are absent, and authored rigid equipment socket transform placement.

Still required:
1. Latest GitHub Actions format/core/desktop/Windows portable validation, including no regressions in 2012 `CryTek 0x745` meshes.
2. **Real Stage 2 and 2012 asset tests:** Kala (torso/equipment DDS), Wraith stages, Griffin and Goliath/Behemoth skin rigs. Record the exact error and original CDF attachment names, matched render-companion virtual paths, number of GPU subset surfaces/textures, bone names and missing dependencies. Do not commit copyrighted extracted assets.
3. When a CDF lacks the true skeleton or rendered compiled equipment companion, the model must report incomplete assembly rather than falsely claim every attachment loaded. Implement original bone-paletted skinning and bone-attached animation only against real skeletons.
4. Current model viewer does **not** automatically infer which CDF owns an arbitrary selected standalone `.skin`; selecting the authored CDF is still the supported full-assembly route. Future discovery should match exact CDF Binding references, not filename similarity.
5. Source MTLs with no original mesh-subset table still cannot safely assign individual texture slots. No guessed face/material association is permitted.

## Follow-up implementation — DBA indexing, multi-material FBX, recovery phases

Work is on PR #6 branch `feature/blender-viewport-cdf-autoload`. **Run the latest core + desktop + portable checks after this commit; earlier green runs do not validate these changes.**

- **Native DBA source index** (`native_dba.rs`): `index_native_dba_bytes` now parses bounded Controller_905 named animation directory records, additive/cycle flags and original controller CRC/track-index references for both standard and in-place streaming ordering. Invalid index pairs, repeated CRCs, track offsets and truncated directory records fail. Synthetic fixtures cover both modes. **No compressed key samples are decoded; native DBA playback and UI integration remain unimplemented.** Compare real DBAs with an independent decoder before claiming format parity.
- **Rigged FBX per-material primitives** (`rig.rs`, `character_unified_fbx.rs`): original `0x08001017` index subset ranges are preserved as separate GLB primitives, sharing original UV0 and JOINTS/WEIGHTS accessors. Each original MTL leaf is matched by material ID, with independently resolved diffuse/normal/emissive/specular/gloss DDS maps embedded into its own glTF material. NoDraw leaves keep zero-alpha materials rather than shifting source subset IDs. Unknown subset 255 or missing/ambiguous exact material source fails instead of painting the entire character with one atlas. Synthetic unit test verifies material index ranges and attributes. **Blender 5.2 real-character reimport still requires material slot count/order, UV seam/alpha, skeleton and animation acceptance tests**, especially Kala equipment and Wraith stages. `fully_textured` remains false until validated.
- **Native offline recovery planning** (`native_offline_launch.rs`): `plan_native_offline_swap` now checks installed signed originals are byte-identical to the precise isolated original PAKs and hashes the planned replacements. `audit_native_offline_swap` read-only classifies `OriginalReady`, `InterruptedBeforeMount`, `OfflineReplacementMounted`, and `StrandedOriginalBackup` from exact hashes; it refuses third-party modification and unsafe/linked files. Synthetic tests create these four states without invoking the game. **Native file swapping, process launch, atomic journal persistence, power-loss-safe restore and offline DLL gating are NOT implemented or enabled.** Do not wire game-launch UI to this until independent crash recovery and offline-only validation pass.

Next implementation checkpoints:
1. Fix all latest GitHub Actions formatting/Clippy/compiler/test failures and run Windows portable CI at the final commit.
2. Exercise synthetic Blender multi-primitive round-trip; verify split meshes/material slots/UVs and source skin weights, then repeat with real Stage 2 and 2012-era assets.
3. Implement and validate actual Controller_905 compressed time/position/quaternion tracks and per-animation frame ranges, including negative-offset mode, against real DBA samples. Only then expose native playback.
4. Implement a durable native swap journal with fsync, atomic rename ordering, idempotent restore and explicit process/watchdog shutdown. Prove failure injection in isolated install clones before touching any installed game.

## October 20 continuation — original MTL slots, native CAF layers, offline launch gate

These commits build on the verified FBX publishing and viewport changes. All changes remain experimental until the newest CI, real Stage 2 assets and Windows launch recovery are independently checked.

**Source multi-material parity**
- `materials.rs` now preserves empty/self-closing `<Material .../>` leaves in their exact original depth-first order, including `Shader="NoDraw"`. The previous parser dropped those leaves, shifting GPU subset ID 1 onto material 0 and potentially putting equipment/glow textures on the wrong character part.
- `assembly_materials.rs` preserves exact source material slot ranges, and `textured.rs` does not draw explicit NoDraw subsets. A valid all-NoDraw table stays hidden instead of rendering the complete model through fallback.
- Synthetic cases exercise NoDraw/empty/body/emissive slots in their original positions. Still unverified: actual Kala multi-material mapping, texture alpha/opacity, LOD subset IDs, per-face UV assignments, and Blender multi-slot FBX export. No fabricated diffuse DDS fallbacks.

**Native CAF/DBA animation parity**
- `caf_layers.rs` now applies CryEngine-style additive **local delta** translations/quaternions in the mixer with an explicitly verified full compiled bind pose. Ordinary CAF layers still use controller-CRC-matched nonadditive interpolation. Weighted additive rotation is composed onto the actual base pose; unknown CRCs, missing/incomplete bind or invalid weights fail.
- The no-bind mixer still rejects additive clips. The source convention is from documented CryEngine animation metadata; synthetic pose tests are not proof all Stage 2 clips obey that exact convention. Native CAF assembly already accepts `mesh_at_layers`; actual GUI selection and layering controls need full live integration/verification.
- **Direct DBA playback is not implemented.** `native_dba.rs` now provides header and named-animation index validation; compressed keyframe sampling is still absent and `playable_animation_count` remains zero. DBA bundles have compressed/shared controller streams and metadata-only success is not playback. Next: use permitted real AnimationsDBA.pak samples to verify chunk formats and storage mode, parse per-animation directory/controller references, decode compressed times/positions/rotations, map original controller CRCs to the compiled skeleton, and compare samples/timing against an independent converter. Do not commit proprietary assets.

**Native offline-launch parity**
- `native_offline_launch.rs` introduces `inspect_native_offline_readiness`, a **read-only** preflight using the exact isolated stage, enabled validated profile, independently signed installed PAKs, safe game executable, distinct original/custom public keys, nonoverlapping roots and no stranded per-target backups. It returns `native_launch_armed=false` by design.
- No installed PAK is copied, swapped, injected or executed by this API. Native installed-game launch/swap, crash-safe backup journal, power-loss recovery, process-close verification, DLL offline gating and restore-watchdog handshake remain **blocking**. The private Python/Frida launcher is still a separate compatibility path and must not be silently treated as a native implementation.
- Next: design an explicit native transaction manifest recording original SHA256, target, temp/backup paths, verified custom digest, restore intent and crash recovery; test simulated mid-copy/mid-rename interruption, symlinks/case aliases, mismatched keys and disk failure in isolated temp game roots. Only after audited recovery and real offline safety should any GUI launch button invoke the native path.

**Verification:** check the latest commit/CI for format, core tests, desktop clippy/tests and portable Windows packaging before marking any of the above green. The previous Wraith Blender 5.2 report is not a checked-in fixture; live game acceptance remains outstanding.

## Exact exporter work left in progress

### What was wrong in the public branch

- `character_unified_fbx.rs` read and verified the **23-byte binary FBX header** from the temporary file, then copied without returning to byte zero. That silently produced a published file missing the complete header.
- The external Blender script's Python errors were not reliably reflected in the process exit status.
- The Blender round-trip's list of MESH objects counted Blender 5.2 **bone-display Icospheres** as character parts; such display objects may legitimately be separate meshes, but only if referenced as custom bone shapes should they be excluded.
- The previous screenshot from a separate working session **reported** that Wraith stage 3 exported with a rig and CAF Action, and that further UV/weight validation and other stages were underway. The screenshot is **not** a persisted test artifact for the current branch; do not claim this report re-tested until reproduced against extracted assets.

### Current code implementation (revalidate on latest CI)

- `crates/evolve-core/src/fbx.rs`: shared `publish_verified_binary_fbx()` checks file type, size and signature, **seeks to byte zero** before a full copy, checks copied byte count, syncs, and publishes atomically with no overwrite. Regression fixtures check exact byte equality and invalid signatures. Both single and unified exporters use this publisher.
- `crates/evolve-core/src/character_unified_fbx.rs`: calls the shared publisher; passes Blender's `--python-exit-code 1`, so script errors should abort the export.
- `crates/evolve-core/src/blender_character_fbx.py`: filters objects that are actually referenced as pose-bone custom display shapes rather than deleting anything named “Icosphere.” Reimport validates original bone hierarchy and source armature binding, verifies finite per-vertex weights referencing actual bones, checks original UV0 loop presence, compares the full sorted per-axis UV-loop coordinate distribution (tolerance 0.001) and overall UV bounds across reimport, and requires a resulting CAF Action when animation is explicitly enabled. Exception handling explicitly exits nonzero.
- No fallback bone assignment, fake UVs, source asset edits, release screenshots, or bundled proprietary game keys have been added.

### Repeatable real-asset acceptance checklist (not yet verified here)

- [ ] From a fresh, separate Windows test location, export a **single unified Wraith stage 3 FBX** with opt-in compatible CAF. Confirm Blender 5.2 exits 0; inspect file's first 23 bytes for `Kaydara FBX Binary 0x00 0x1A 0x00`, and confirm the file can be reopened.
- [ ] Reimport in a clean Blender 5.2 session: exactly the intended character rig, unchanged bone hierarchy, no bone-display Icosphere falsely classified as an exported character mesh, UV0 present and aligned, nonzero source bone weights, and an editable original CAF Action with real pose changes.
- [ ] Test Wraith stages 1–2 and compare to stage 3; test hunter **Kala** with equipment; then **Goliath/Behemoth** for compound monster rigs. Test both animation-disabled (smaller file) and opt-in animation cases.
- [ ] Exercise 2012 `CryTek 0x745` assets separately from `CrChF v7` Stage 2; keep the historical geometry, UV0 and bone compatibility independent. Current automated legacy fixture only establishes bounded mesh decoding.
- [ ] Check separate diffuse/normal/emissive maps, multi-slot materials, transparency/glow, original UV layout and source LOD. Do **not** mark `fully_textured` true until source-authored multi-slot material parity is verified.
- [ ] Force invalid CAF, missing compatible skeleton, bad UV/weight source, Blender script failure, timeout and existing destination: none should publish a partial FBX or change original files.
- [ ] Record **exact asset identifiers**, Blender version, mesh/bone/weighted vertex/Action counts, and source/output hash *locally*; no real assets or sensitive paths should be committed.

### Remaining gaps and likely next implementation

1. **FBX:** Verify actual Blender 5.2 round-trip with Wraith stages and Kala. Verify per-face UV seam corner assignment (the current complete UV-coordinate distribution catches value loss but cannot detect swapping UVs among faces), and ensure split meshes preserve skin weights and material slots. Extend the exporter beyond the present single-slot authored material limitation; preserve fully populated multi-part equipment and correct CA_BONE attachment placement. Stage character variants intentionally; confirm the whole character has one armature and correct action(s). Ensure Blender stdout/stderr diagnostic capture is bounded and does not leak private paths to public logs.
2. **Character viewport:** Finish real-stage tests of CDF per-part materials, 2012 legacy model loading, GPU lighting/grid and bone-mounted equipment motion. Current native CAF playback rejects unverified skin/bone references, by design. Fully animated layered CAF clips need coherent source bone mapping.
3. **CAF/DBA:** Native DBA packed bitstream decoding, catalog search, idle auto-selection for all valid characters, layered/additive CAF playback and exporting multiple verified FBX Actions are still incomplete.
4. **Offline mod launch:** Isolated signed-PAK staging and journaled rollback exist. Native installed-game mount/swap, process launch, verified offline safety, emergency recovery and production profiles are not parity-complete; no private launcher script/key should be included in the public repo.
5. **Secondary parity:** Full Level Editor scene dependencies, original GFX/USM editing, installer/updater upgrade acceptance, asset-library performance and other GUI controls remain release gates in PARITY.md.

## Last known evidence, provenance and limits

- Before this continuation, PR #6 HEAD was `59a6b1bd69a510f1c5677022f7cb8f5e056bd6f1` and Actions run `39372627121` reported **success**. This was **before** the exporter changes above.
- A separate previous working session (screenshot, not checked-in test output) reported real Blender 5.2 Wraith stage 3 plus CAF Action, but work was interrupted at its usage limit; it named header publication, Blender exit propagation and display-Icosphere classification as blockers. Its local tests, temp files and screenshots are **not automatically available** on this branch.
- For the current changes, trust only CI at the latest PR head and new independently recorded local Blender tests. Every unresolved checkbox remains a blocker, not an implicit success.

## Operating constraints

Keep the public repository independent of the private manager's proprietary assets, credentials and personal install paths. Keep documentation focused on implementation status, evidence, and reproducible verification. Keep release assets limited to the requested portable application and installer; development artifacts are not a release. No merge/tag until required checks and real-game acceptance are complete.
