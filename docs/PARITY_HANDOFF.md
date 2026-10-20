# Public Rust manager parity — continuation notes

Updated: **2026-10-20** · Repository: `corvodl/Evolve-Mod-Manager` · Working PR: **#6** on `feature/blender-viewport-cdf-autoload`, based on PR #5's `feature/pr5-combined-viewer-preview`. This document is a resumable engineering checklist, **not** a claim of production parity. See [PARITY.md](PARITY.md) for the broader feature matrix.

## Resume here first

1. Inspect PR #6's latest commit, GitHub Actions **core**, **desktop** and **Windows portable preview** conclusions. Do not assume an older green CI result covers the newest commit.
2. Run `cargo fmt --all -- --check`, `cargo test -p evolve-core --all-targets`, `cargo clippy -p evolve-core --all-targets -- -D warnings`, `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace --all-targets`. Fix failing tests without disabling lint or source validation.
3. Resume **real Blender 5.2 character acceptance** for unified FBX, then native character CAF/DBA and offline installed-game workflows. Any real extracted assets remain out of Git and release packages.
4. Update this file after each verified advance with the exact commit/run, observation, blockers, and next operation. Keep PR open until representative real-asset acceptance and CI pass; no release just for green synthetic tests.

## Exact exporter work left in progress

### What was wrong in the public branch

- `character_unified_fbx.rs` read and verified the **23-byte binary FBX header** from the temporary file, then copied without returning to byte zero. That silently produced a published file missing the complete header.
- The external Blender script's Python errors were not reliably reflected in the process exit status.
- The Blender round-trip's list of MESH objects counted Blender 5.2 **bone-display Icospheres** as character parts; such display objects may legitimately be separate meshes, but only if referenced as custom bone shapes should they be excluded.
- The previous screenshot from a separate working session **reported** that Wraith stage 3 exported with a rig and CAF Action, and that further UV/weight validation and other stages were underway. The screenshot is **not** a persisted test artifact for the current branch; do not claim this report re-tested until reproduced against extracted assets.

### Current code implementation (revalidate on latest CI)

- `crates/evolve-core/src/fbx.rs`: shared `publish_verified_binary_fbx()` checks file type, size and signature, **seeks to byte zero** before a full copy, checks copied byte count, syncs, and publishes atomically with no overwrite. Regression fixtures check exact byte equality and invalid signatures. Both single and unified exporters use this publisher.
- `crates/evolve-core/src/character_unified_fbx.rs`: calls the shared publisher; passes Blender's `--python-exit-code 1`, so script errors should abort the export.
- `crates/evolve-core/src/blender_character_fbx.py`: filters objects that are actually referenced as pose-bone custom display shapes rather than deleting anything named “Icosphere.” Reimport validates original bone hierarchy and source armature binding, verifies finite per-vertex weights referencing actual bones, checks original UV0 loop presence, checks round-trip UV bounds (allowing topology splits), and requires a resulting CAF Action when animation is explicitly enabled. Exception handling explicitly exits nonzero.
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

1. **FBX:** Verify actual Blender 5.2 round-trip with Wraith stages and Kala. Compare UV seams, not only UV envelopes, and ensure split meshes preserve skin weights and material slots. Extend the exporter beyond the present single-slot authored material limitation; preserve fully populated multi-part equipment and correct CA_BONE attachment placement. Stage character variants intentionally; confirm the whole character has one armature and correct action(s). Ensure Blender stdout/stderr diagnostic capture is bounded and does not leak private paths to public logs.
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
