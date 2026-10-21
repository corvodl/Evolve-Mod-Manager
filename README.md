# Evolve Mod Manager

An experimental Windows modding and asset-inspection tool for **Evolve Stage 2**, built with Rust, Iced, and wgpu.

> **Nightly 0.1.0 is a development preview.** Some features are unfinished, and not all models, animations, materials, or exports are game-accurate yet. The first full 1.0 release is still in development.

## Download and install

**[Download the latest release from GitHub Releases](https://github.com/corvodl/Evolve-Mod-Manager/releases)**

Choose one of the two Windows downloads attached to a release:

- **Installer** — `EvolveModManager-Setup-*.exe`. Run the installer and choose whether to add a desktop shortcut, a Start Menu shortcut, and launch the manager after installation.
- **Portable** — `EvolveModManager-Portable-*.zip`. Extract the ZIP to a folder of your choice, then run `evolve-mod-manager.exe`. No installation is necessary.

Windows may request administrator approval when launching the manager. Download builds only from the repository's Releases page. You **do not** need to install Rust, Visual Studio, or build the project yourself.

Nightly releases receive experimental fixes first. Under **Settings → Updates**, choose Stable, Beta, or Nightly and check for new versions. Updates are verified against the SHA-256 checksums included in their GitHub release notes.

## Features

The following features are available or being actively tested in the public preview:

- **Modding workspace** — File browsing, asset inspection, PAK import and isolated mod staging with backup/recovery safeguards.
- **PAK tools** — Inspect and extract CryEngine archives, including supported signed/encrypted Stage 2 PAKs; experimental archive rebuilding and signing workflows.
- **Asset Library** — Import assets into a searchable workspace and preview them without editing the original archive.
- **GPU model viewer** — Preview supported `.cgf`, `.chr`, `.skin`, `.skinm`, and `.cdf` models, with camera controls and character attachment assembly.
- **Textures and materials** — Preview DDS textures and authored diffuse, normal, and emissive maps; export supported DDS images to PNG at their highest available resolution.
- **Character effects** — Preview selected CryEngine glow materials and animated emissive Decals, including Kala's Banshee Gun; exact in-game HDR/bloom parity is still in progress.
- **Animations** — Experimental native CAF playback and character animation inspection, with limited DBA/conversion support.
- **Blender exports** — Experimental model, character-assembly, and rigged FBX export, with optional material textures and animation support where verified.
- **Level tools** — Preview supported terrain and level assets and export experimental Blender-compatible scenes.
- **Settings and updates** — Appearance options, configurable paths, recovery preferences, and separate Stable/Beta/Nightly update channels.

**Compatibility varies by asset.** Some Stage 2 and historical assets require additional material or skeleton handling. Missing dependencies and ambiguous references are reported rather than replaced with invented data.

## Planned additions and improvements

- More accurate **character and monster previews**, including full CDF attachments, higher-fidelity materials, source UVs, emissive effects, and in-game-style bloom.
- Improved **Blender exports** with reliable rigs, skin weights, animations, high-resolution textures, and complete monster/hunter assemblies.
- More complete **CAF/DBA animation** playback, skeleton compatibility, and layered animations.
- Expanded **Level Editor** tools for assembling, editing, and exporting textured levels.
- More complete **PAK and mod-profile management**, safe installed-game application, restore/rollback, and automated dependency handling.
- **Seamless in-app updates** that apply verified downloads and restart the manager without making the user run the installer manually.
- Performance, stability, and usability improvements as Nightly builds are tested.

See [the development parity checklist](docs/PARITY.md) for technical implementation status.

## Notes

This project is an independent community tool. Work with copies of game assets when testing experimental modification features. Previewing or exporting assets does not imply they can be safely used in an installed game, and modding may disable online functionality. Preserve backups and follow applicable game and community rules.

The original game data, extracted assets, private signing keys, and personal configuration are **not included** with the downloads.
