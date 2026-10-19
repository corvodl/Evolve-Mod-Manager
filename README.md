# Evolve Mod Manager

Windows desktop modding toolkit — independent Rust rewrite using **Iced** and **wgpu**.

> **Status: early development (0.1.0). NOT a 1.0 release.** This repository is a new public codebase with its own history. The existing production manager's capabilities must be reimplemented and verified before a public 1.0 can replace it.

## What works today

- Native Iced desktop shell using the wgpu GPU renderer.
- Explicit local-directory scanning, read-only bounded inventory of CryEngine assets.
- Basic file-type classification for PAK, CDF, skin, mesh, material, split DDS, CAF, DBA, GFX and levels.
- Searchable inventory list and safe JSON report export from the CLI.
- Automated Rust unit tests and Windows compile checks.

**Not implemented yet:** PAK decryption/extraction/rebuilding/signing, game launch and restore, DDS decoding, model/texture/animation viewing, FBX exports, Blender integration, map editor, updater and Windows installer. Do not use this preview as a replacement for the existing manager.

## Build (Windows)

Install Rust (stable) and Visual Studio C++ Build Tools with a Windows SDK, then run:

```powershell
cargo run -p evolve-mod-manager
```

To inspect an explicitly chosen directory without running the GUI:

```powershell
cargo run -p evolve-core --bin evolve-cli -- scan "C:\\path\\to\\extracted-assets" "inventory.json"
```

The output JSON contains relative file names, sizes and recognized types. It contains no game assets, but can still reveal local filenames; don't publish it without reviewing.

## Development contract

- Maintain Windows as the primary release target.
- No proprietary game files, signing keys, account data, or injected binaries in this repository.
- All CryEngine parsers must reject malformed sizes, offsets, dependencies and paths before writes.
- Preserving model skeletons, original UV0 and texture quality takes priority over a visually plausible but incorrect conversion.
- Verified CAF/DBA Actions are **optional**, with animation-free FBX exports the default when the exporter is implemented.
- Publish 1.0 only after the parity matrix and release checklist in [docs/PARITY.md](docs/PARITY.md) are satisfied.
- Decide and publish the project's license before distributing the public release.

The Iced backend currently uses wgpu 27, matching the Iced 0.14 renderer dependency to avoid duplicate incompatible GPU APIs.
