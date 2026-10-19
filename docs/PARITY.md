# Evolve Mod Manager — Rust 1.0 feature-parity gate

This table tracks **actual Rust implementation state**, not feature mockups. The existing production manager's behavior is the acceptance baseline. Real proprietary test assets must stay outside this public repository.

| Feature | Rust rewrite status | 1.0 acceptance |
| --- | --- | --- |
| Local indexed file browser and deterministic inventory | **Partial: searchable, folder-grouped Asset Viewer; Shift/Ctrl selection in viewer, Modding projects, library catalog and Level outliner (native Windows validation pending)** | Fast search for realistic libraries, virtualized GUI, no freezing |
| Historical ZIP-compatible PAK inspect/extract/rebuild | **Implemented subset** | Original archive untouched, bounded CRC-verified extraction, file-path safety |
| Stage 2 signed/encrypted CryPAK RSA/Twofish | **Experimental: verify and extract implemented, real-game unverified** | Verify signatures and central-directory/local offsets, native decrypted streaming |
| Signed Stage 2 PAK editing and repacking | **Experimental: 1–64 raw or converted CryXmlB members in a single staged write/sign operation, with final per-member verification**; real-game acceptance still unverified | Golden round-trips and signature verification after patch injection |
| Staged offline mods, backups and crash-safe restore | **Experimental isolated workspace**: single-PAK and multi-PAK profile staging, before/after SHA-256, journals, recovery tests, CLI/GUI controls; no installed-game launch integration | Journaling and rollback after cancel, crash and power loss |
| Asset Library multi-PAK resolution and provenance | **Partial exact CDF resolver in CLI/GUI, now automatic when previewing a CDF from an indexed folder**: missing/ambiguous paths reported, not guessed; no full cross-PAK provenance/assembly graph | Cross-PAK matching only when exact and unambiguous |
| DDS/streaming top mip and BCn → PNG | **Implemented subset** | Golden pixel comparisons, signed BC5, BC6/BC7, console endian/split variants |
| CDF XML skeleton/attachment parsing | **Implemented subset** | CDF variants, skin sets, dependency graph and verified meshes |
| CryEngine skin/CHR/CGF/skinm geometry parser | **Partial: bounded CryTek/CrChF geometry and conditional verified native bone/weight parsing**; unsupported layouts fail closed | Bone hierarchy, weights, bind pose, LOD, UVs and rig integrity |
| Original UV0 and highest-quality FBX output | **Partial: UV0-preserving OBJ/GLB, rigged single-mesh GLB and Blender FBX bridge with verified skeleton/weights; optional explicitly selected highest-mip DDS diffuse embedded in FBX through GLB**; complete materials/real character parity unverified | Independent Blender verification; no invented UVs |
| Monster stages, hunter 1P+3P and equipment | **Partial: exact CDF dependency-based, source-space multi-mesh static assembly previews and virtual Full Character entries; no verified live skeleton deformation or full material slots** | One painting-ready root FBX with exact per-variant rigs |
| Optional verified CAF/DBA animation FBX Actions | **Partial: explicit non-additive native CAF matched to original skeleton CRCs, optional glTF animation channels into Blender FBX; automated full-character CAF/DBA collection and real-game Actions validation missing** | Disabled by default; validated opt-in; smaller FBX test |
| Animation browser and layered timeline | **Partial: native CAF controller decoding/sampling, converted DBA/GLB sampling and GUI scrub/play; no verified skeleton deformation/layers** | Skeleton matching, CAF/DBA clips, scrub and idle |
| GPU model viewer and CryEngine materials | **Partial: Iced/wgpu mesh with original UV0 and diffuse/normal/emissive DDS; read-only authored .mtl parser, exact/ambiguous map resolution, automatic single-slot texture loading; no full multi-slot material/shader parity or skinning** | Real wgpu mesh, high-res DDS, normal/emissive/transparency parity |
| Level Editor and Blender scene export | **Partial: terrain, XML/LYR object placement, geometry-scene GLB and Blender FBX through converter; incomplete map textures and dependencies** | Correct complete-scene placements, terrain, textures and dependencies |
| GFX, USM and loose-file editing | **Not ported** | Real-asset read/preview/edit/repack integrity |
| Settings, mod profiles, signed updater and installer | **Partial: persisted preferences, verified public update checks and installer, isolated signed-PAK mod profile catalog/planning and batch stage transactions; production game profiles and upgrade tests incomplete** | Versioned releases, recovery, private-data exclusion, tested installs and rollback |
| Responsive UI and safe long-running jobs | **Partial: detached Activity monitor with live task status and optional automatic opening, plus opt-in read-only System Health, immediate single-click asset preview and LMB/Shift/RMB Blender-like viewport navigation; GUI frame/Windows interaction verification pending** | Cancellation, progress, large-input and Windows GUI tests |

### Binary XML reader compatibility

CDF, MTL, chrparams animation catalogs and level placements now accept original
CryXmlB documents as well as text XML. The shared decoder checks file/table
bounds, table overlap, UTF-8/XML characters, attribute uniqueness, parent/child
agreement, reachability and expansion/depth limits. Level transform export can
read a binary source and write a separate text XML copy; it leaves the original
unchanged and refuses to overwrite an existing destination.

Synthetic fixtures generated with the reference manager's encoder exercise all
four readers, XML escaping and malformed inputs. The Rust encoder matches the
reference synthetic binary fixtures and preserves XML semantics, namespaces,
attribute order and escaped whitespace. It rejects unsupported mixed content,
comments and processing instructions rather than silently dropping them.
Forty representative real documents encoded by Rust passed semantic comparison
through the reference decoder, with all source hashes unchanged. Complete
character rigs/materials and real-game acceptance remain outstanding.

### October 19 source-data compatibility corrections

- Static CDF assembly now matches selected skin-set attachment names to their
  corresponding binding paths; previously those different identifiers were
  compared directly and valid parts were skipped.
- Level import and transform export distinguish ID-only entity links inside
  object `Entities` lists from placed instances. Links are preserved unchanged;
  duplicate actual placement IDs still fail validation.
- Authored texture references with repeated interior separators normalize to
  the same virtual path. Rooted paths and traversal remain rejected.
- Editable project explorers expose the existing verified original-file restore
  operation, retaining edited revisions and reindexing the project afterward.
  Protected Asset Library views cannot invoke it.

Read-only local acceptance checked 655 CDFs, 582 chrparams catalogs, 455 level
layers and 9,853 CryEngine materials successfully. Ten additional `.mtl` files
are Wavefront format and remain unsupported by this CryEngine material reader.
All 11,555 checked sources retained their SHA-256 hashes. Forty real-source
documents re-encoded locally with the reference encoder produced equivalent
reader results to their text forms. These are parser/transform checks, not
rendering, skeletal animation, signed-gameplay or installer acceptance.

## Release requirements

1. All important binary operations must have safety and parity tests, bounded memory allocations and safe path handling.
2. Validate archives before any writes; modifications must be staged and rollback-capable.
3. Support Stage 2 signed/encrypted PAKs and the original modding workflow, not just ZIP-style archives.
4. Import complete CDF assemblies with correct original UVs, materials, rigs and bones.
5. Export actual FBX through a Blender-compatible path, including optional verified animations and full character folders.
6. Validate model/level viewing and editor behavior using actual permitted game assets.
7. Complete the Windows installer/portable ZIP, updater, packaging integrity and source/license review.
8. Pass complete Windows CI and manual real-game tests before tagging `v1.0.0`.

Do not convert an internal Rust prototype into a public production release just because core unit tests pass.

## October 2026 technical milestones (experimental)

- **Signed Stage 2 PAK inspection/rebuild/staging**: external user-provided RSA public and private keys, exact signed basename, custom Twofish CTR streaming, RSA-PSS verification, capped DEFLATE output and CRC checks. One-member re-signed rebuild is independently verified; synthetic signed-PAK staging and restore tests cover a complete cycle. **No independent real Stage 2 PAK acceptance verification** and no installed-game swap integration. Never ship game key material.
- **Geometry**: bounded CryTek/CrChF source streams, UV0 and normal preservation, geometry OBJ/GLB output, strict native bone/weight verification for supported skinm/skeleton pairs, and Iced/wgpu GPU mesh preview with authored .mtl-driven diffuse/normal/emissive DDS for unambiguous single-slot materials. Not complete material or skeletal animation parity.
- **FBX**: external Blender translates validated GLB into binary FBX; original single-mesh rigged exports require matching compiled skeleton and weights. An explicit original DDS diffuse and original compatible non-additive CAF may be embedded with validated source UV and bone CRC checks. Automated character animation libraries, full multi-part CDF rigs, full texture slots, and DBA/layered Actions remain **not** equivalent to the private manager.
- **Animations**: native CAF controller decode/sampling, chrparams indexes, converter-backed DBA/GLB preview, and an experimental one-clip matched-CAF rigged FBX embedding pipeline. Raw DBA decode, automated per-character catalog/clip embedding, reliable skinned viewport playback and layering remain unfinished.
- **Levels**: Heightmap.dat terrain decoding, mission/LYR hierarchy and transforms, non-destructive edits, geometry-scene GLB and Blender FBX bridge exposed in Iced. Full material/geometry dependency assembly, normal maps and real-map scene parity remain unported.
- **Public output**: no user-owned game files, keys, private repository history, secrets or proprietary assets should be committed. Public release tag 1.0 must be gated by full actual parity.

## Asset Library visual parity: persistent imported archive browser

- Split Asset Library and Modding into separate Iced pages. Archive rebuilding, signing keys, isolated staging and mod profiles are **not** displayed in the read-only Asset Library; they remain on Modding.
- The Asset Library now follows the established page structure: File/View/Tools strip, Import PAKs and Refresh actions, All Assets / By Build tabs, search and snapshot filter, a five-column Dependency Group / Folder / Description / File / Extracted table, selection, footer totals and Preview Files.
- The viewer reads existing `Data/AssetLibrary/*/asset-library.json` manifests and `build-collections.json` read-only, with a bounded background indexer and fixed-size table pages. It can auto-detect the established Windows per-user Asset Library folder. No PAK decryption or permanent content import is silently triggered.
- Preview Files verifies that each manifest-listed file still resides in its imported archive folder with its expected length, then opens the established separate native Asset Viewer window; original library files stay protected.
- **Editable project copy (restricted):** the library's `Edit / Add to Mod Project` action now checks that the selected read-only import refers to an **exact, actual PAK under the configured stage/paks**. It re-verifies library entries and creates an independent new editable project by safely re-extracting the authenticated staged source; it never edits Library files and never guesses an archive from a matching basename. The private manager's optimized copy-on-write mapping, cross-build imports and full provenance are not yet ported.
- **Cross-PAK texture lookup (experimental):** exact virtual-path matching is available across verified imported archive manifests; source .tif/.dds references prefer a matching .dds.0 highest-resolution stream. A selected build snapshot scopes matches; duplicates remain explicitly ambiguous. The single-slot wgpu material preview may load a uniquely matched imported DDS only when its local root lacks that authored map. Full per-part CDF materials, multislots, source SHA audit on every lookup and GPU skinned assemblies are still pending.
- **New experimental import:** the Asset Library screen accepts an explicit signed/encrypted Stage 2 PAK path, discovers its RSA public key from the configured stage, verifies/decrypts/CRC-checks a fresh isolated extraction, converts bounded CryXmlB members to text XML, hashes each extracted member, and publishes a private-compatible version-one `asset-library.json` only after its source remains unchanged. A repeated import verifies existing file hashes before reporting already imported. The original PAK and other library folders are never edited. **Ordinary/historical ZIP PAK import, batch progress/cancellation, and a real-source game acceptance run remain missing.**

## Modding page parity — public Rust preview

The page now mirrors the private Qt manager's **project-first workflow** instead of exposing unrelated cryptographic or staging forms by default:

- Native-style File/Edit/View/Tools contextual command strip, searchable Projects table (project, target, changed, build, state, details), selected project's modified-files panel, and compact Install/Profiles/Refresh/Continue/Build actions.
- Advanced PAK browser collapsed by default, with read-only staged archive selection, signature verification, Level Editor and a *specialist* single-member replacement panel.
- Reads private-manager `mod-projects.json` and `.evolve-pak-workspace.json` without changing them, bounds path traversal and avoids following symlinks, compares changed files against manifest hashes in a background worker.
- `Unpack PAK` verifies the selected source with the installed stage's public key and creates a new editable workspace under Projects. Representable CryXmlB members become text XML with verified original backups; other members retain their raw bytes.
- Signing resource paths now resolve only from the configured existing stage's `mykeys/public_key.bin` and `mykeys/private_key.pem`; the signed basename comes from the source PAK, and a target may only come from an exact staged archive or selected verified project. Missing keys are setup errors, not silently fabricated credentials.
- `Build Mod` can rebuild **1–64** exact, validated modified workspace members into one isolated signed output. It copies the source payload once, appends all replacements, signs once and verifies every replacement. Converted XML is serialized against the authenticated original member with source-hash and root-name checks. Imported replacements use an independently selected local file and a SHA-verified `EditorBackups` revision, without altering the original signed source. Synthetic tests cover stored/compressed mixed batches, converted XML, unchanged members and original preservation. Complete dependency extraction, the original GUI's one-click installer, full profiles, installed-game swap integration and real-game acceptance are **not yet parity complete**.
- The UI no longer requests PEM, DER or basename manually. No private keys are included in the repository, executable or update downloads.

## Rust/Iced/wgpu interactive GUI improvements (development preview)

The following are **new implementation work**, not a claim of complete Qt parity:

| Capability | Implemented in the preview | Still required |
|---|---|---|
| GPU character preview | Existing source-textured wgpu renderer, wireframe switch, camera persistence, left/middle-drag orbit, Shift+drag/right-drag pan and scroll zoom, GPU-backed UV0 visualization preserving tiling; explicit CDF static assembly preview | Per-part GPU selection, skinned 3D animation, multiple material slots, full CDF equipment composition, real-time shadows and faithful transparency |
| Asset Library responsiveness | Catalog filtering cached on search/build changes; bounded page rendering instead of generating all rows, background index and scanned-file progress | Truly virtual scrolling, thumbnail workers, 40 GiB stress benchmarks and sustained frame-time tests |
| CAF/DBA animation timeline | Real sampled CAF or validated converted GLB scrubbing, speed range 0.1–3.0×, loop control and saved animation options | DBA direct playback, layered track mixer, verified bone bindings and actions applied to skinned meshes |
| Level Editor | GPU-backed 2D placement overview from source transform coordinates, point-picking, hierarchy, object details, zoom and bounded culling | GPU instanced 3D geometry, terrain material rendering, object transform gizmos, full editor undo/redo |
| Workspace layout | Separate native viewer window, adjustable explorer ratio, saved camera/panel and playback settings | Fully detachable and dockable panels, independent multi-tab workspaces, complete keyboard shortcuts |
| Asset dependency explorer | Authored CDF and MTL dependency edges rendered in a node diagram with found/missing/ambiguous diagnostics | Multi-hop PAK/material/animation edges, per-node navigation and automatic correction tools |
| Original vs modified compare | Read-only complete SHA-256 and DDS dimension comparison | Visual pixel/UV/geometry diff modes |
| Shared GPU asset memory | Per-process bounded in-memory DDS LRU reused across windows, source metadata invalidation and retained Arc image ownership | Multi-device GPU texture pool, cache telemetry and profiling |
| Background jobs | Worker thread operations; cancellable read-only directory indexing with exact file counts | Progress/cancel/rollback support per archive export/rebuild and all operations |
| Drag/drop and diagnostics | Native file/folder drop dispatch with protected Asset Library context and inline inspection statuses | Drag-out, batch selections, all-format previews |
| Appearance | Private-like charcoal Home cards, compact controls and red primary action accents | Full per-widget theme audit, focus accessibility and actual-frame rendering comparisons |

All source libraries stay read-only. New PAK installation/rebuilding must keep its existing transaction and restore checks. Missing features remain gated and must be tested against representative Stage 2 assets before public 1.0.

- **GUI styling progress:** applied subdued charcoal surfaces, neutral library/tool buttons and compact neutral sidebar selection, leaving the red accent for primary actions. Sidebar icon labels remain hover-only; genuine screenshot-based Windows comparison with the private Qt executable is still needed. The settings icon and responsive layout should be verified by screenshots in the built Windows app.
- **Home:** a collapsed System Health disclosure now allows on-demand, **read-only** inspection of configured game/stage/project paths, process presence and limited firewall rule information. It does **not** change Windows settings or claim feature parity for dump setup, firewall actions or cleanup.
- **Activity:** separate nonmodal window with bounded task-status history and scan cancellation; honors the optional automatic-open preference and can be closed without cancelling work. Real full workflow progress indicators are still incomplete.
- **Viewer:** selecting a supported source file launches its preview without an extra Preview click. CDF rows start the separately verified static assembly preview. Selection modifiers remain separate from previews. Both wgpu/canvas viewports accept LMB and middle-drag orbit, Shift-drag/right-drag pan, and wheel zoom. Full per-part rigging remains incomplete.

## Desktop GUI parity — private manager reference

- The private v1.3.5 manager is the design reference: dark charcoal surfaces, compact red-accent icon rail, top breadcrumb, Home hero/status dashboard, Instructions, Asset Library, Modding, What's New, Credits and password-gated Developer. Bottom rail is Play with Mods, Play without Mods and Settings. Playback shortcuts deliberately remain disabled until the installed-game launch and recovery implementation is verified.
- Iced 0.14 uses a multi-window daemon. The **Asset Viewer** opens as a separate native OS window, with a dark menu bar (File/Edit/View/Model/Textures/Animation/Tools/Help), wgpu preview, camera controls, compact native animation timeline, searchable bounded File Explorer on the right, and a status row. Original Asset Library files remain read-only.
- Supported source .skin/.skinm/.chr/.cgf/.cdf/.dds/.dds.0/.caf/.mtl types can be inspected from an indexed extracted directory. The file explorer groups actual CDF definitions under virtual Full Character entries; opening one requires exact skeleton and compatible skin paths before showing an **unrigged source-space composite preview**. PAK central-directory rows still cannot be previewed without extraction.
- Implemented UI controls deliberately do not promise missing engine capabilities. **Still missing**: full CDF character assembly/equipment/gpu skeletal playback, map-based file tree and per-project editing workflow, complete native animation catalog and layer editor, GFX workbench, confirmed in-game launch/restore, production event tooling, and exhaustive original-asset tests.
- Potential Rust advantages over the legacy Qt implementation: one typed state reducer for all windows, background Tokio workers for decompression/indexing, wgpu reuse across previews, and bounded filtered explorer rendering. Virtualized rows, retained geometry/texture caches and multi-asset tabs require further benchmarks; they are not claimed as shipped.

## Isolated mod profiles and recovery (development only)

- Persistent versioned `profiles.json` catalogs live inside a newly initialized offline staging workspace. They store only file references and SHA-256 hashes; they do not copy assets or keys into the Git repository. CLI and Iced support creation, listing, signed-file registration, verifying enabled mods and resolving duplicate targets.
- Each signed PAK's **own** authenticated basename is used when planning mixed-archive profiles. A source must independently verify against the explicitly provided user-supplied RSA public key; changed hashes and missing/ambiguous targets fail closed.
- A multi-PAK profile can be applied to its **isolated staging workspace** and restored using a separate recoverable batch journal. Original PAK bytes are preserved and verified before the first modification; single-file transactions are prohibited during an active batch. This does **not** change installed game files.
- There is no production user-facing mod installation/profile activation yet. Full parity still requires integration with the established game launch/swap workflow, separate backup volume handling, cancellation/progress, offline/online DLL rules, validation with representative original Stage 2 data, installer upgrades and game startup acceptance tests.

## Windows packaging and settings milestone (experimental)

- Rust settings load the established user preferences (including backup, density, file explorer side, theme fields, data paths and update options). Unrecognized older keys are preserved; imported legacy settings are read without modifying original data.
- Stable/experimental update checks run against the new public releases repository, with a checksum-confirmed installer download and a separate approval action before launch. Actual upgrade/rollback behavior must still be validated on Windows.
- The versioned installer and portable ZIP are the **only** intended public release assets. The installer offers an optional desktop shortcut, installs per-user and preserves private user files. A manually approved release workflow stays blocked by unfinished parity markers.
- Application screenshots are diagnostic resources and are **not** packaged in GitHub Releases.

## Native viewer automatic dependency loading (development)

- On a single-click model preview, the wgpu mesh is shown first, then an
  asynchronous read-only source discovery examines indexed extracted assets and
  the manifest-verified Asset Library. Previously decoded character maps are
  cleared immediately; a queued different selection cannot inherit them.
- The resolver looks for a matching same-character CryEngine material and loads
  only uniquely resolved authored maps (diffuse, normal and emissive, currently
  one material slot). A documented/highest cooked DDS `.dds.0` stream is
  preferred. Conflicting build/library paths are reported, never guessed.
- An exact `.chrparams` tied to the selected character populates a browser of
  declared CAF, DBA and blendspace references. A suitable non-contextual idle
  may be decoded automatically, while other explicitly declared CAFs can be
  resolved from local or imported manifests on click. The original PAKs and
  protected Asset Library remain untouched.
- A matching same-character CDF may supply a verified skeleton for the existing
  rigged FBX export preparation. It is not a substitute for validating the
  selected mesh's skin weights, original UVs and skeleton compatibility.

**Not yet private-manager parity:** compiled-model material references and
CDF attachment material overrides, multiple shader/material slots, animated
GPU skinning, complete multi-part equipment assemblies, DBAs and layered
animation playback, full-character FBX folder/action exports, game-level
texture effects, and native export dialogs/workflow. CAF controller decoding
does **not** imply its pose is applied to the rendered mesh. Real Stage 2
character, Windows GUI and Blender acceptance remains a release gate.


## October 19 — bounded CAF pose layer foundation

The Rust core now has a separately tested `sample_verified_caf_layers` routine.
It blends at most eight **decoded, non-additive CAF** layers by exact bone
CRC on a caller-supplied original skeleton identity set, normalizing quaternion
results and rejecting unknown/duplicate controllers, invalid weights, missing
base transform channels and incompatible additive input. Layer data remains
read-only; this does **not** mean live skinned viewport animation, a direct
DBA decoder, imported character bind-pose reconciliation, or multi-action FBX
embedding has been implemented. Those remain 1.0 release blockers.

Complete private-manager parity additionally requires verified CDF per-part
materials and rigs, all animation database paths, layered GPU deformation,
atomic flat whole-character FBX exports with correctly embedded assets, and
native right-click file operations; these are **not complete** in the public
rewrite. Test each using real permitted Stage 2 assets and Blender before
changing these labels or authorizing v1.0.
