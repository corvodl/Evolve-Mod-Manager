# Standalone sparse mod PAK export

From **Modding**, select an editable project, choose its mod name, then click
**Export Mod as zz_ PAK…** (also available from the Edit menu). A native
**Save As** dialog allows any existing destination folder and a custom
`zz_[modname].pak` filename. This is independent of the existing Build Mod
and Play with Mods/PAK swap workflows; it neither installs nor enables mods.

## Contents and signing

- Only the 1–64 **verified changed project members** are packaged, not the
  original archive's unchanged files. Registered additions are included where
  supported by the current signed build backend. Unregistered files are ignored.
- The normal build pipeline first validates the project against its authenticated
  source PAK and serializes CryXML and other edited assets. This produces an
  intermediate complete signed PAK, not the final export.
- The authorized official Stage 2 tool decrypts that signed intermediate and
  signs a fresh PAK from a directory containing **only changed members**. The
  new `zz_` output basename is used for signing; simply renaming an already
  signed original PAK would fail basename authentication.
- Before publication, the manager independently checks the signed PAK under
  its new filename, exact changed member list, size and CRC32 for every member,
  and decrypted bytes for members up to 64 MiB. It refuses unsafe filenames,
  duplicate/case-colliding entries, and an existing output file.
- Custom name is 1–80 ASCII letters, digits, spaces, underscores or hyphens;
  spaces become underscores. A `zz_` prefix is not doubled.
- The official developer encoder must be present and match the configured
  signing stage. This PR contains no developer executable or private key.
  The fallback locally configured PEM path is not used for sparse exports.

## Scope and acceptance

One project is exported per operation. A project can target one original PAK,
and this version does not combine multiple projects. The user chooses the
destination; the original game, recovery journal and Mod Project contents stay
unchanged. A cache or isolated intermediate full PAK may be written inside
the Modding build area as part of the already existing verified build process.

The reported `zz_` load-order override is based on user testing. Validate
the exported addon in-game on the supported Windows build: replacement wins
over the original when loaded, no original archive is replaced, removing the
addon restores stock content, and signature verification survives copying the
exported file to its final location. Proprietary Windows encoder and hook
compatibility are not exercised by standard hosted CI.
