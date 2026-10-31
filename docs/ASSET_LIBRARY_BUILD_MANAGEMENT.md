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

## Manage Game Builds popup and identity migration

**Asset Library → Manage Game Builds** opens a floating modal, leaving the
archive browser and its scroll position unchanged. It lists incremental
registered installations and legacy `build-collections.json` groups. Each
entry provides a friendly **Rename** action and a confirmed removal action;
the toolbar can refresh registrations and import/update a selected installation.

The first explicit Manage Game Builds refresh creates or updates
`managed-builds.json` as a **non-destructive migration index**. It records a
stable path-normalized build ID, its prior history-folder IDs and the optional
friendly name. A normal Windows drive path and the `\\\\?\\` verbatim
spelling map to the same logical build. Existing history JSON, original PAKs
and extracted content are not moved or rewritten by this migration. Update
scans append new snapshots to the stable identity and respect the latest
previous snapshot across old folder aliases.

Imported archives use ordinary path spellings in new manifests, and the
read-only catalog hides *byte-identical, same-source* duplicates when the
original name, SHA-256 and extracted member counts/bytes match. It never
merges different game roots or conflicting archive bytes. The Level Editor
can resolve exact same-installation geometry even where stale duplicate build
labels existed. Truly missing CGFs, unknown DDS formats and unsupported baked
terrain tile formats remain separate rendering issues.

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

Incremental build removal deletes the selected logical group's validated
historical snapshot folders (including validated alternate Windows-path IDs)
and only provably manager-owned, unshared extractions. Legacy collection
removal deletes **only that collection's registration** from
`build-collections.json`; extracted imports, other collection members and
the protected `unassigned` bucket remain intact. Renaming either kind changes
only its label. Disk estimates are based on recorded extracted asset sizes,
not full filesystem allocation accounting.

## Testing and remaining limits

Tests cover a relocated build reusing two identical archives while importing
one new PAK, deletion preserving a shared extraction and both installed
source PAKs, and a rejected stale confirmation. Windows CI and manual
full-game acceptance are required before release. SHA-256 scans still need to
read source PAKs once; the optimization avoids redundant disk **copies and
decompression**, not initial content verification.


### Ownership guard for deletion

Only extraction directories that the managed update actually **created** are
marked reclaimable. An archive imported manually before a build was registered
is preserved even if that build later references it. Snapshots created before
this ownership metadata was available are still removable as build metadata,
but their unproven extraction directories are deliberately retained. The
UI reports only safe, provable reclaimable storage.
