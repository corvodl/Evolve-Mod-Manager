# Optional independent Cryengine-Converter (GPLv2)

Cryengine-Converter is a **separate command-line executable**, not linked into
Evolve Mod Manager. It is not required to build or run the manager. This setup
does not contain game files, extracted assets, or private tools.

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

Manual installer builds include the staged converter as an **unchecked**
optional task; the portable ZIP includes it only if staging was requested
before packaging. On GitHub Actions, `include_cryengine_converter` is an
explicit opt-in input for manual Nightly or production release dispatch;
ordinary Nightly request pushes never fetch it. Keep the source archive and
notice next to the binary in any redistribution, and comply with GPLv2.
If distributing a changed binary, this pin and its corresponding source must
be updated together, with licensing reviewed again.
