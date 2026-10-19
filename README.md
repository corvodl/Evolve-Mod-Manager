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

**Experimental modules on this branch:** signed Stage 2 PAK RSA-PSS verification, Twofish decrypt/extract, and single-member rebuilding with user-provided custom keys; isolated PAK staging with verified import, undo journal and recovery; CryTek/CrChF geometry with preserved UV0 and strict single-mesh rig checks; explicit source DDS diffuse/normal/emissive preview through wgpu; exact CDF dependency resolution; native CAF sampling and converter-backed DBA/GLB clips; Blender GLB-to-FBX conversion (verified animations **OFF by default**); Level Editor terrain/LYR geometry assembly and Blender GLB/FBX output. Real-game validation is still required.

**Not yet at feature parity:** installed-game mod injection/rollback and multi-mod profiles, complete CDF hunter/monster assemblies and validated FBX skeleton/UV/material fidelity, automatic MTL resolution, skinned GPU playback, native DBA decoding/layering, complete textured Blender level scenes, GFX editor, and real-game updater/installer validation. Signed-PAK operations remain offline with independently supplied keys; synthetic success is not proof that the game will load a rewritten archive. This preview is **not** a replacement for the current production manager.

## Persistent settings and verified updates (experimental)

The Windows desktop stores preferences under `%LOCALAPPDATA%\\EvolveModManager\\settings.json`, independent of where the executable runs. If no Rust settings file exists, it reads the installed manager's former `Data/Settings/manager_settings.json` without changing or deleting that original file. Recognized settings and unknown older keys are preserved.

The Settings page offers the established dark/light appearance, compact density, file-explorer position, reduced motion, backups, activity log, workspace and data paths, Blender executable, and stable/experimental update options. Some controls persist their values while their dependent workflows remain under development.

Automatic or manual checks use only public GitHub Releases. Newer releases must supply exactly a verified installer and portable ZIP, with SHA256 checksums in the release description. The manager verifies a downloaded installer and requires an additional user action to launch it. These features require final Windows integration testing.

An Inno Setup script installs per-user under LocalAppData with the original icon and optional desktop shortcut. `.github/workflows/release.yml` is manually triggered and **refuses to publish** while the 1.0 parity checklist contains unfinished capabilities. Any public release is restricted to these two download files:
- `EvolveModManager-Setup-VERSION.exe`
- `EvolveModManager-Portable-VERSION.zip`

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
cargo run -p evolve-core --bin evolve-cli -- cdf resolve "C:\Assets\character.cdf" "C:\Assets" "character-dependencies.json"
cargo run -p evolve-core --bin evolve-cli -- model glb "C:\Assets\model.cgf" "C:\Exports\model.glb"
cargo run -p evolve-core --bin evolve-cli -- terrain obj "C:\Assets\Heightmap.dat" "C:\Exports\terrain.obj"
cargo run -p evolve-core --bin evolve-cli -- level inspect "C:\Assets\mission_mission0.xml" "level.json"
cargo run -p evolve-core --bin evolve-cli -- signed inspect "C:\Archives\libs.pak" "C:\Keys\original-public.der" "libs.pak" "verified.json"
cargo run -p evolve-core --bin evolve-cli -- stage init "C:\Exports\NewOfflineStage"
cargo run -p evolve-core --bin evolve-cli -- stage import "C:\Exports\NewOfflineStage" "C:\Archives\libs.pak" "Game/libs.pak" "C:\Keys\original-public.der" "libs.pak"
cargo run -p evolve-core --bin evolve-cli -- stage status "C:\Exports\NewOfflineStage"
# After creating and verifying a separately signed replacement with a matching filename:
# evolve-cli stage apply <stage> <replacement.pak> <Game/libs.pak> <original-public.der> <new-public.der> <libs.pak>
# evolve-cli stage restore <stage>
cargo run -p evolve-core --bin evolve-cli -- fbx export "C:\Exports\verified.glb" "C:\Program Files\Blender Foundation\Blender 4.2\blender.exe" "C:\Exports\body.fbx"
```

Extraction output must be an **absent folder**; PNG and JSON outputs must be **new files**. Rebuilds refuse existing outputs. Signed Stage 2 rebuilding and staging are limited experimental offline workflows with user-provided RSA keys: there is no installed-game swap or automatic profile manager. Never use the installed game directory as the isolated stage, and restore any pending journal before applying another replacement.

## Public release policy

- Windows is the primary target. The codebase has independent public Git history.
- Do not commit original game PAKs, extracted game assets, keys, personal settings, DRM bypass material, or user-specific files.
- Preserve exact high-resolution textures, original UV0, skeleton and skin weight relationships. Missing/ambiguous references must be reported rather than guessed.
- The planned complete-character FBX exporter must preserve hunter 1P/3P equipment and all three independently rigged monster stages. Verified animations must be **optional and OFF by default**, matching the existing manager's latest merged behavior.
- Publish `1.0.0` only once every requirement in [docs/PARITY.md](docs/PARITY.md) is implemented and passes synthetic regression, real Windows build, and real-game compatibility testing.
- Select and publish a license before distributing binary builds.

This new public project does not disclose the private development repository history.
