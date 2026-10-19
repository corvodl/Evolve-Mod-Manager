# Evolve Mod Manager

Independent Windows desktop modding toolkit rewrite using **Rust**, **Iced 0.14**, and **wgpu 27**.

> **Development preview: 0.1.0 — NOT a 1.0 release.** Only the implemented components listed here work. Porting and validating the complete feature set from the existing manager is an ongoing prerequisite for the first public 1.0.

## Currently implemented in Rust

- Responsive Iced dark desktop shell with navigation, searchable file inventory, PAK member browsing, DDS metadata inspection, and explicit output fields.
- Bounded read-only scanning of local CryEngine assets, classification by extension, deterministic results, and an optional JSON inventory report.
- **Historical ZIP-compatible PAK** inspection, extraction, and rebuild from existing-member replacements (stored and DEFLATE methods), with bounded sizes, unsafe-name and symlink rejection, duplicate detection, CRC-checked streaming, and newly created staging/output folders. Original PAK files are never modified.
- **DDS texture inspection and highest-resolution PNG export** for BC1, BC2, BC3, unsigned BC4/BC5, and common 32-bit RGBA/BGRA. Supports complete `.dds`, complete `.dds.0`, and fully extracted consecutive `.dds.0`–`.dds.N` mip fragments. BC5 is exported as raw red/green channels rather than simulated normal-map shading.
- **CDF XML character-definition inspection**, retaining explicit skeleton, CA_SKIN/CA_SKIN_UNIQUE attachments, and skin-set references. An ambiguity-aware asset resolver refuses to guess among duplicate workspace references.
- Off-thread filesystem operations in the GUI, Rust unit tests, and Linux + Windows CI checks.

**Not yet implemented:** Stage 2 signed/encrypted PAK decryption, signature verification, re-signing or repacking; installed-game modification or mod launch/restore; CDF geometry and skinning; GPU mesh rendering; CAF/DBA animation decoding; Blender/FBX/level export; GFX editor, profiles, updater and Windows installer. This preview is **not** a replacement for the current production manager.

## Build on Windows

Install current stable Rust, Visual Studio C++ Build Tools with Windows SDK, then:

```powershell
cargo run -p evolve-mod-manager
```

Standalone command-line equivalents:

```powershell
cargo run -p evolve-core --bin evolve-cli -- scan "C:\Assets" "inventory.json"
cargo run -p evolve-core --bin evolve-cli -- pak inspect "C:\Archives\Legacy.pak" "pak-info.json"
cargo run -p evolve-core --bin evolve-cli -- pak extract "C:\Archives\Legacy.pak" "C:\Exports\LegacyExtracted"
cargo run -p evolve-core --bin evolve-cli -- pak rebuild "C:\Archives\Legacy.pak" "C:\Overrides" "C:\Exports\LegacyRebuilt.pak"
cargo run -p evolve-core --bin evolve-cli -- dds info "C:\Assets\body.dds.0"
cargo run -p evolve-core --bin evolve-cli -- dds png "C:\Assets\body.dds.0" "C:\Exports\body.png"
cargo run -p evolve-core --bin evolve-cli -- cdf inspect "C:\Assets\character.cdf" "character-info.json"
```

Extraction output must be an **absent folder**; PNG and JSON outputs must be **new files**. The operations refuse existing outputs rather than overwriting them. Replacement files must match existing members of an ordinary ZIP PAK. Encrypted Stage 2 PAK support remains unfinished.

## Public release policy

- Windows is the primary target. The codebase has independent public Git history.
- Do not commit original game PAKs, extracted game assets, keys, personal settings, DRM bypass material, or user-specific files.
- Preserve exact high-resolution textures, original UV0, skeleton and skin weight relationships. Missing/ambiguous references must be reported rather than guessed.
- The planned complete-character FBX exporter must preserve hunter 1P/3P equipment and all three independently rigged monster stages. Verified animations must be **optional and OFF by default**, matching the existing manager's latest merged behavior.
- Publish `1.0.0` only once every requirement in [docs/PARITY.md](docs/PARITY.md) is implemented and passes synthetic regression, real Windows build, and real-game compatibility testing.
- Select and publish a license before distributing binary builds.

This new public project does not disclose the private development repository history.
