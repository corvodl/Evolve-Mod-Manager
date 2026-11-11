# Export a Mod Project to a signed PAK

On the **Modding** page, select an editable Mod Project and click
**Export Mod…** (also in the **Edit** menu). Select one of three formats,
then use the native Windows **Save As** dialog to choose any existing
destination folder.

| Choice | Filename | Included archive members |
| --- | --- | --- |
| Changed addon | `zz_[modname].pak` | Only verified changed files |
| Changed original-name | `[OriginalPakName].pak` | Only verified changed files |
| Entire original PAK | `[OriginalPakName].pak` | All original files, with changes applied |

The first option uses the project's **Mod name** as the suggested filename.
You can choose another valid `zz_` name in Save As; names accept ASCII
letters, digits, spaces (converted to underscores), underscores, and hyphens.
The latter two options must preserve the exact physical filename of the
project's original PAK. They may be exported to any other folder, but cannot
be renamed in Save As without changing the signed archive's identity.

## What happens

- Each option runs the existing authenticated **Build Mod** validation and
  serialized asset output, including registered additions when the current
  backend supports them. Unchanged/unregistered loose project files are
  excluded from changed-only exports.
- The two **changed-only** formats require the authorized official Stage 2
  encoder. An isolated complete signed build is decoded, only the changed
  members are copied to a new temporary folder, and the official encoder
  signs that filtered folder using the **chosen output basename**. The
  manager verifies the new PAK's signature, exact changed-member list,
  sizes/CRC32 and smaller decrypted members before publishing it.
- The **entire original PAK** format copies the already rebuilt and signed
  archive with the original filename. Signature and SHA-256 copy identity
  are verified before publication. This retains the existing signing-backend
  fallback where the official encoder is not configured.
- Exports use a create-only publication and refuse any existing output file,
  including concurrent overwrite attempts.
- A single selected Mod Project is exported per operation. Export does not
  install or enable a mod, change original game PAKs, modify launcher
  selection, or bypass the swap/recovery journal.
- Pending GFX scratch edits must be applied to the Mod Project first.

## Compatibility and acceptance testing

The reported `zz_` addon load-order precedence is based on user discovery.
**Windows and in-game verification are still required.** Confirm that each
mode is accepted by the official Stage 2 tool and the supported Evolve build,
that unchanged members are absent from both changed-only exports, and that
the entire original-name export includes unchanged members. In particular,
a changed-only PAK **with the original filename** might require different
mount ordering or might shadow the stock PAK; do not assume it behaves the
same as the `zz_` additive override until tested on game files. Confirm
reversible removal and no change to the installed stock archives.

This PR includes no proprietary encoder binaries, signing keys or game assets.
