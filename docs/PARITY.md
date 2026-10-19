# Evolve Mod Manager — Rust 1.0 feature-parity gate

This table tracks **actual Rust implementation state**, not feature mockups. The existing production manager's behavior is the acceptance baseline. Real proprietary test assets must stay outside this public repository.

| Feature | Rust rewrite status | 1.0 acceptance |
| --- | --- | --- |
| Local indexed file browser and deterministic inventory | **Initial implementation** | Fast search for realistic libraries, virtualized GUI, no freezing |
| Historical ZIP-compatible PAK inspect/extract/rebuild | **Implemented subset** | Original archive untouched, bounded CRC-verified extraction, file-path safety |
| Stage 2 signed/encrypted CryPAK RSA/Twofish | **Experimental: verify and extract implemented, real-game unverified** | Verify signatures and central-directory/local offsets, native decrypted streaming |
| Signed Stage 2 PAK editing and repacking | **Experimental: exact one-member replacement, re-signing with a user-supplied RSA key and independent verification**; no game-acceptance validation | Golden round-trips and signature verification after patch injection |
| Staged offline mods, backups and crash-safe restore | **Experimental isolated workspace**: single-PAK and multi-PAK profile staging, before/after SHA-256, journals, recovery tests, CLI/GUI controls; no installed-game launch integration | Journaling and rollback after cancel, crash and power loss |
| Asset Library multi-PAK resolution and provenance | **Initial exact CDF resolver in CLI and GUI**: missing/ambiguous matches are reported, not guessed; no full provenance graph | Cross-PAK matching only when exact and unambiguous |
| DDS/streaming top mip and BCn → PNG | **Implemented subset** | Golden pixel comparisons, signed BC5, BC6/BC7, console endian/split variants |
| CDF XML skeleton/attachment parsing | **Implemented subset** | CDF variants, skin sets, dependency graph and verified meshes |
| CryEngine skin/CHR/CGF/skinm geometry parser | **Partial: bounded CryTek/CrChF geometry and conditional verified native bone/weight parsing**; unsupported layouts fail closed | Bone hierarchy, weights, bind pose, LOD, UVs and rig integrity |
| Original UV0 and highest-quality FBX output | **Partial: UV0-preserving OBJ/GLB, rigged single-mesh GLB and Blender FBX bridge with verified skeleton/weights**; real character parity unverified | Independent Blender verification; no invented UVs |
| Monster stages, hunter 1P+3P and equipment | **Not ported** | One painting-ready root FBX with exact per-variant rigs |
| Optional verified CAF/DBA animation FBX Actions | **Policy default only** | Disabled by default; validated opt-in; smaller FBX test |
| Animation browser and layered timeline | **Partial: native CAF controller decoding/sampling, converted DBA/GLB sampling and GUI scrub/play; no verified skeleton deformation/layers** | Skeleton matching, CAF/DBA clips, scrub and idle |
| GPU model viewer and CryEngine materials | **Partial: Iced/wgpu mesh with original UV0 and diffuse/normal/emissive DDS; read-only authored .mtl parser, exact/ambiguous map resolution, automatic single-slot texture loading; no full multi-slot material/shader parity or skinning** | Real wgpu mesh, high-res DDS, normal/emissive/transparency parity |
| Level Editor and Blender scene export | **Partial: terrain, XML/LYR object placement, geometry-scene GLB and Blender FBX through converter; incomplete map textures and dependencies** | Correct complete-scene placements, terrain, textures and dependencies |
| GFX, USM and loose-file editing | **Not ported** | Real-asset read/preview/edit/repack integrity |
| Settings, mod profiles, signed updater and installer | **Partial: persisted preferences, verified public update checks and installer, isolated signed-PAK mod profile catalog/planning and batch stage transactions; production game profiles and upgrade tests incomplete** | Versioned releases, recovery, private-data exclusion, tested installs and rollback |
| Responsive UI and safe long-running jobs | **Partial** | Cancellation, progress, large-input and Windows GUI tests |

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
- **FBX**: external Blender translates validated GLB into binary FBX; original single-mesh rigged exports require matching compiled skeleton and weights. Optional verified animations remain OFF by default. Full multi-part CDF rigs/stages/equipment and animation Actions are **not** equivalent to the private manager yet.
- **Animations**: source CAF metadata, native CAF controller decoding and time sampling, chrparams indexes, converter-backed DBA clips and GLB preview. Raw DBA decode, verified bone matching and skinning playback, animation layering and FBX Actions remain unfinished.
- **Levels**: Heightmap.dat terrain decoding, mission/LYR hierarchy and transforms, non-destructive edits, geometry-scene GLB and Blender FBX bridge exposed in Iced. Full material/geometry dependency assembly, normal maps and real-map scene parity remain unported.
- **Public output**: no user-owned game files, keys, private repository history, secrets or proprietary assets should be committed. Public release tag 1.0 must be gated by full actual parity.

## Asset Library visual parity: persistent imported archive browser

- Split Asset Library and Modding into separate Iced pages. Archive rebuilding, signing keys, isolated staging and mod profiles are **not** displayed in the read-only Asset Library; they remain on Modding.
- The Asset Library now follows the established page structure: File/View/Tools strip, Import PAKs and Refresh actions, All Assets / By Build tabs, search and snapshot filter, a five-column Dependency Group / Folder / Description / File / Extracted table, selection, footer totals and Preview Files.
- The viewer reads existing `Data/AssetLibrary/*/asset-library.json` manifests and `build-collections.json` read-only, with a bounded background indexer and fixed-size table pages. It can auto-detect the established Windows per-user Asset Library folder. No PAK decryption or permanent content import is silently triggered.
- Preview Files verifies that each manifest-listed file still resides in its imported archive folder with its expected length, then opens the established separate native Asset Viewer window; original library files stay protected.
- **Still unfinished:** a new-PAK import wizard that generates permanent library manifests, full browse-by-build management, large-list virtualization, editable project copy workflow and source-hash audit on every preview. The Import PAKs control currently exposes the library-folder setup/help panel; it must not be reported as an implemented source archive importer. The Edit / Add to Mod Project action stays disabled until copy-on-write project creation is supported.

## Desktop GUI parity — private manager reference

- The private v1.3.5 manager is the design reference: dark charcoal surfaces, compact red-accent icon rail, top breadcrumb, Home hero/status dashboard, Instructions, Asset Library, Modding, What's New, Credits and password-gated Developer. Bottom rail is Play with Mods, Play without Mods and Settings. Playback shortcuts deliberately remain disabled until the installed-game launch and recovery implementation is verified.
- Iced 0.14 uses a multi-window daemon. The **Asset Viewer** opens as a separate native OS window, with a dark menu bar (File/Edit/View/Model/Textures/Animation/Tools/Help), wgpu preview, camera controls, compact native animation timeline, searchable bounded File Explorer on the right, and a status row. Original Asset Library files remain read-only.
- Supported source .skin/.skinm/.chr/.cgf/.cdf/.dds/.dds.0/.caf/.mtl types can be inspected from an indexed extracted directory. PAK central-directory rows are not directly previewed until extracted, and no separate character full-body assembly is manufactured from a single mesh.
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
