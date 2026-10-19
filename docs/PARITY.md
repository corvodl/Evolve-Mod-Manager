# Evolve Mod Manager — Rust 1.0 feature-parity gate

This table tracks **actual Rust implementation state**, not feature mockups. The existing production manager's behavior is the acceptance baseline. Real proprietary test assets must stay outside this public repository.

| Feature | Rust rewrite status | 1.0 acceptance |
| --- | --- | --- |
| Local indexed file browser and deterministic inventory | **Initial implementation** | Fast search for realistic libraries, virtualized GUI, no freezing |
| Historical ZIP-compatible PAK inspection/extraction | **Implemented subset** | Original archive untouched, bounded CRC-verified extraction, file-path safety |
| Stage 2 signed/encrypted CryPAK RSA/Twofish | **Not ported** | Verify signatures and central-directory/local offsets, native decrypted streaming |
| PAK editing, changed-only rebuild and custom signing | **Not ported** | Golden round-trips and signature verification after patch injection |
| Staged offline mods, backups and crash-safe restore | **Not ported** | Journaling and rollback after cancel, crash and power loss |
| Asset Library multi-PAK resolution and provenance | **Initial resolver** | Cross-PAK matching only when exact and unambiguous |
| DDS/streaming top mip and BCn → PNG | **Implemented subset** | Golden pixel comparisons, signed BC5, BC6/BC7, console endian/split variants |
| CDF XML skeleton/attachment parsing | **Implemented subset** | CDF variants, skin sets, dependency graph and verified meshes |
| CryEngine skin/CHR/CGF/skinm geometry parser | **Not ported** | Bone hierarchy, weights, bind pose, LOD, UVs and rig integrity |
| Original UV0 and highest-quality FBX output | **Not ported** | Independent Blender verification; no invented UVs |
| Monster stages, hunter 1P+3P and equipment | **Not ported** | One painting-ready root FBX with exact per-variant rigs |
| Optional verified CAF/DBA animation FBX Actions | **Policy default only** | Disabled by default; validated opt-in; smaller FBX test |
| Animation browser and layered timeline | **Not ported** | Skeleton matching, CAF/DBA clips, scrub and idle |
| GPU model viewer and CryEngine materials | **Not ported** | Real wgpu mesh, high-res DDS, normal/emissive/transparency parity |
| Level Editor and Blender scene export | **Not ported** | Correct complete-scene placements, terrain, textures and dependencies |
| GFX, USM and loose-file editing | **Not ported** | Real-asset read/preview/edit/repack integrity |
| Settings, mod profiles, signed updater and installer | **Not ported** | Versioned releases, recovery, private-data exclusion and tests |
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
