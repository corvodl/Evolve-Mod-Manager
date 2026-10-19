# Evolve Mod Manager — Rust 1.0 parity gate

These are release requirements, not claims about current implementation. The existing Windows manager is the behavioral baseline. Tests must use permissioned user-provided samples or copyright-safe synthetic fixtures; none belong in the public repository.

| Feature | Rust rewrite status | 1.0 acceptance |
| --- | --- | --- |
| Local indexed file browser and deterministic inventory | Implemented (initial) | Fast search at realistic archive sizes, no UI blocking |
| Signed and encrypted CryPAK parsing / verification | Not ported | Validate original signatures/offsets without corrupting archives |
| Twofish/decompression, extraction and repacking | Not ported | Golden round trips and malformed archive fuzz tests |
| Staged mod injection, offline safety, restore journals | Not ported | Transactional restore after cancellation, process crash and power loss |
| Asset Library multi-PAK resolution and provenance | Not ported | Only unambiguous exact assets selected; cross-PAK dependencies preserved |
| DDS / DDS.0 highest mip, normal maps and material channels | Not ported | Fidelity and pixel checks against verified reference images |
| CryEngine CDF assembly and skeletal geometry | Not ported | Correct rig, bone hierarchy, skin weights and attachments |
| Highest-quality FBX and original UV0 export | Not ported | Blender 4.2+ import/export roundtrip and independent mesh validation |
| Monster stages / hunter 1P+3P equipment | Not ported | One painting-ready root scene with correct independently rigged components |
| Optional verified CAF/DBA animation Actions | Not ported | Disabled by default; enabled only when validated; measurable FBX size reduction |
| Animation browser and model-viewer timeline | Not ported | Skeleton matching, scrub, idle and layered animation fixtures |
| GPU model viewer with CryEngine materials | Not ported | Wgpu-rendered mesh, high-quality textures and material/normal parity |
| Level Editor and Blender scene export | Not ported | Complete-scene material/dependency and placement verification |
| GFX and loose-file editing | Not ported | Read, preview, edit and roundtrip validation |
| Settings, backup profiles, updater and installers | Not ported | Signed builds, rollback, no credential leakage, stable/beta updates |
| Resizable panes, off-thread indexing and responsiveness | Partial | Windows end-to-end GUI tests and meaningful load-time benchmarks |

## Production release requirements

1. Build a small, auditable Rust core before porting risky binary manipulation.
2. Keep a file-format compatibility test corpus outside the public repo.
3. Separate UI rendering from background parsing; enforce cancellation and progress reporting.
4. Verify every file writing operation transactionally, with checksum and recoverability checks.
5. Build real Windows installer and portable ZIP and verify their contents.
6. Test real-game model rendering, rigging and exports in Blender, with animation OFF and ON.
7. Run full CI plus manual Windows GPU, crash recovery, and realistic large-library tests.
8. Never describe unported capabilities as supported or publish the 1.0 release prematurely.

The public repository must not contain private source-history metadata, developer credentials, or proprietary game assets.
