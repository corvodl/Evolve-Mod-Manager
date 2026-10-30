# Managing imported Evolve game builds

This workflow manages **Asset Library snapshots**, not game installations or
mod projects. Use **Asset Library → Update Game Build…** to select the
installed EvolveGame build folder and compare its PAKs before importing.

## Incremental imports, including moved game installations

- The scanner calculates source PAK SHA-256 hashes without writing the game.
- If the same source-root-relative PAK is unchanged, it is skipped.
- If the file is new to the selected root but the Asset Library already has
  an imported PAK with the **same name and source SHA-256**, the accepted
  extraction is **reused** across both build snapshots. It is not copied or
  extracted again, even when the installed game moved to another directory.
- Only new or changed PAKs whose exact hashes are not already in the
  Asset Library require authenticating/extracting. A change within an encrypted
  PAK generally requires re-extracting that PAK, not the entire game.
- Preview reports **new**, **changed**, **unchanged**, **reused**, and bytes
  that actually require importing. The confirmation step re-hashes every
  changed/reused source PAK and compares its planned identity before use.
- Previously signed PAK imports retain their existing verification rules.
  Reusing an already imported identical source PAK never downgrades signing
  verification requirements.
- Imported build snapshots record source-root identity and references.
  Distinct game directories remain separate builds even when they share
  extracted source data.

## Deleting old imported builds safely

Use **Asset Library → Manage Game Builds** to view managed build snapshots,
associated PAK counts, and potentially reclaimable extracted bytes.
Select **Delete Imported Build**, then explicitly **Confirm Delete**.
The operation removes that build's managed snapshot history and unshared,
manager-created extraction folders only.

It **never deletes** original PAKs from EvolveGame, test installations,
signed stage copies, independent mod projects or archive extractions still
referenced by another build (including legacy collections). An imported
archive whose ownership/provenance is uncertain is retained. Cleanup errors
are reported, and the library is reindexed afterward.

This panel currently manages **incremental build-update snapshots**.
Legacy `build-collections.json` groups and unassigned manual archive imports
are deliberately left alone rather than silently deleting potentially shared
data. Disk estimates are based on recorded extracted asset sizes, not full
filesystem allocation accounting.

## Testing and remaining limits

Tests cover a relocated build reusing two identical archives while importing
one new PAK, deletion preserving a shared extraction and both installed
source PAKs, and a rejected stale confirmation. Windows CI and manual
full-game acceptance are required before release. SHA-256 scans still need to
read source PAKs once; the optimization avoids redundant disk **copies and
decompression**, not initial content verification.
