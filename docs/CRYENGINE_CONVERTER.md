# Bundled independent Cryengine-Converter (GPLv2)

Cryengine-Converter is a **separate command-line executable**, not linked into
Evolve Mod Manager. The Windows Nightly, production installer, and portable
package stage it automatically with the corresponding GPLv2 source archive and
attribution. No separate converter installation is required for supported DBA
conversion. The manager itself can still compile without the converter, but
DBA conversion is unavailable if the independent binary was deliberately
omitted from a custom development build. This setup contains no game files,
extracted assets, or private tools.

For local Windows builds, either provide both downloaded upstream files:

```powershell
./scripts/prepare-cryengine-converter.ps1 -BinaryPath C:\path\cgf-converter.exe -SourceArchivePath C:\path\Cryengine-Converter-source-v2.0.0.zip
```

or explicitly opt in to fetching them from upstream:

```powershell
./scripts/prepare-cryengine-converter.ps1 -Fetch
```

The script pins Markemp **v2.0.0**, source commit
`6ae79ce6aeb669b632ce9adee452a6ad99e1f883`, and executable SHA256
`75feca2813a6517cad9a8e5df28687860a3b0bab9dcbcfeef65e56850d53b0b7`.
It rejects a binary with any other digest and checks that the source archive
has the pinned revision layout, source files, and a GPL version 2 LICENSE.
The source archive is retrieved from GitHub by commit (its bytes are not
independently hash-pinned); inspect the upstream revision if source provenance
must be independently verified. The script does not execute downloaded files.

Successful staging produces only `dist/third_party/Cryengine-Converter/` with
the executable, matching source ZIP, and `NOTICE.txt`. Existing staging is
never overwritten automatically. `dist/` is ignored by Git; do not commit
converter binaries, local game data, or extracted assets.

Release workflows always stage the pinned executable and matching source in
`dist/third_party/Cryengine-Converter/`. The Inno Setup installer installs them
inside `{app}\\third_party\\Cryengine-Converter` by default, and the portable
ZIP includes the same directory. The Model Viewer auto-detects its installed
copy on startup for the DBA browser and manual animation conversion. No
`PATH` modification or separate user download is needed.

This is still an **independent GPLv2 program**, not part of the Rust binary.
Every package must retain the upstream source ZIP and notice beside the EXE.
GPLv2 distribution obligations apply. If distributing a modified binary, its
corresponding source, hash pin, and documentation must be updated together.
