# Official Stage 2 developer toolchain: integration assessment

> Source: developer-provided `evolve_tool_stage2_02.05.2025.zip` (the runtime is packaged from the authorized private tool repository). The manager integration is implemented; the executable and in-game mixed-key behavior still require Windows/runtime validation. No redistribution permissions beyond the authorized release workflow are implied.

## Implementation (PR #101)

The desktop manager now supports the developer's official **2025** encoder as an
pinned toolchain supplied by the authorized release installer/portable ZIP, with local-tool fallback for development.

1. Install a Nightly that includes the official runtime. The installer and
   portable updater place it under `third_party/OfficialStage2` automatically.
   Existing independent installations can instead use
   `<Mod Data Location>/official-stage2-tools/evolve_tool_stage2`, or set
   `EVOLVE_STAGE2_TOOLS_DIR` to the directory containing `evolve_tool.exe`.
2. Keep the original game/client installation intact. In particular, the
   official tools currently require an existing original
   `bin64_SteamRetail/dbghelp.dll` so a complete, reversible swap is possible.
3. Choose **Set Up Manager** with the game and storage root. The current
   setup uses a **sparse signing stage**: it verifies and hashes the installed
   PAK inventory without copying or re-signing every archive. Permanent
   original snapshots are optional and **off by default**.
4. Import/edit a PAK through the normal Modding page. **Build** calls the
   official `d` and `e` commands for the **specific PAK containing changed
   assets**, preserves its other members, authenticates the signed output and
   reuses a SHA-keyed cache if the inputs have not changed.
5. **Play with Mods** provisions and stages only the enabled profile's changed
   PAKs. The automatic launcher writes an exact `active_mod_paks.json` filter
   and the recovery helper swaps only those PAKs plus the required official
   `inject.dll` and `dbghelp.dll`. Unmodified stock PAKs are left in place.
   An older manual stage with no selection manifest may still use its legacy
   full-stage swap, so the sparse behavior is guaranteed for the
   manager-controlled enabled-mod workflow, not arbitrary legacy stages.
6. **Prepare Offline Test** accepts an already-signed official-tool output
   using its original and stage **public verification keys**; it does not
   require a Rust-managed private PEM. The official encoder is still required
   when building changed contents.

**Compatibility caveat:** the supplied official tool's `r` command and
runtime use a replacement signing key and injector. The sparse launcher
*selects* only changed archives, but successful prelaunch signature checks do
not establish that every combination of new-key modded and untouched
stock-key PAKs will mount in the game. Verify on a disposable Test Game;
do not automatically re-sign the entire game or silently fall back to the
legacy full-swap path when a selection is invalid.

**Never** run the official `r` command directly against your installed game. The manager applies it only to isolated copies. The source ZIP and C# projects remain private; authorized release packages carry the verified signing executable and runtime DLLs.

### Validation not yet performed

Windows build/CI checks are required. The provided binaries have not been
executed in this environment, and in-game acceptance, alternative engine builds
and the DLL compatibility offset need testing before a release. The tool may
return success even if a particular PAK failed; the integration therefore
requires independent RSA verification and preserves failures for inspection.

## Complete installer and updater packaging

The developer's extracted tool files have been uploaded to the private
repository `corvodl/Evolve-Official-Tools`, under `evolve_tool_stage2/`.
**A GitHub Release in the private repository is no longer required.**

Nightly builds clone that private repository using a read-only credential,
check the SHA-256 of each required file, and copy only the five runtime
files and the BouncyCastle license into the release staging directory.
The source projects and developer archive are not included in the release.

The installer and portable ZIP package the verified runtime at
`third_party/OfficialStage2/`. The portable updater recognizes the fixed
nested runtime file list and replaces it through its ordinary recovery-safe
update transaction.

### Remaining Actions credential setup

In the **public** `corvodl/Evolve-Mod-Manager` repository, configure
an Actions secret named `EVOLVE_STAGE2_TOOLS_TOKEN`: a fine-grained GitHub
personal access token with `Contents: Read` permission specifically on
`corvodl/Evolve-Official-Tools`. The ChatGPT GitHub connection can read
the private repository, but that does not authorize GitHub Actions in
another repository. Do not put the token itself in a PR, repository
variable, source file, issue, or Actions output.

`EVOLVE_STAGE2_TOOLS_REPO` and `EVOLVE_STAGE2_TOOLS_TAG` are no longer
needed: the workflow uses the verified repository's current default branch,
not a private release tag. The pipeline deliberately fails if its token
cannot fetch the files, if a component is missing, or if its checksum
differs from the pinned 2025 build. It must not publish a Nightly
silently omitting official signing.

A working signing executable includes recoverable signing material. Hash
verification proves integrity and **does not** make that key secret.

The manager-controlled runtime swap and original-file restoration remain
in place; initial setup records an authenticated inventory rather than
converting all stock PAKs, and edited PAKs are provisioned on demand.

## Verified capabilities and constraints

- `evolve_tool.exe d <pak> <dir>` decrypts and unpacks a PAK; `evolve_tool.exe e <dir> <pak>` packs a **complete** directory as one PAK. These are not in-place single-member editing commands.
- `evolve_tool.exe r <game dir>` traverses the existing PAK library and changes each archive's authenticated trailer to use the **new** RSA keypair, without necessarily rewriting encrypted member payloads.
- The instructions require the supplied `inject.dll` and `dbghelp.dll` in `Bin64_SteamRetail`. The inject assembly has a game-executable-specific hook offset that needs validation for each supported build.
- The C# source contains an embedded RSA signing key. Public release bundles contain only the approved runtime executable and dependencies, and do not include the developer's source project, debug artifacts or original ZIP.
- The original Stage 2 verification key in this source is **public**, not the original private signing key. A PAK signed with the new private key cannot be assumed compatible with the unmodified stock verifier. Mixing stock old-key archives and new-key mod archives is unverified.

## Current selective-signing workflow and limitations

1. Setup verifies stock PAK signatures and SHA-256 identities, without
   duplicating or re-signing the whole installed PAK library.
2. When a project changes a member, the manager authenticates the stock
   source, materializes only that PAK into an isolated stage, and converts
   that working copy with the official tool if needed.
3. `evolve_tool.exe d` unpacks the **entire selected PAK**; the manager
   overlays only changed members and invokes `e` to sign one resulting PAK.
   It checks the full original member inventory, unmodified member CRC/size
   and changed member bytes before publication. This does **not** mean that
   `e` encodes just one member inside a PAK.
4. The cache key includes the original PAK digest, signing public-key digest,
   authenticated basename and changed member hashes. Unchanged projects
   reuse verified signed outputs.
5. The enabled-mod launcher selects only edited target PAKs for swapping,
   plus the two official runtime DLLs. Original installation PAKs outside
   the selected set are neither copied nor replaced.
6. The existing workflow rejects two enabled projects targeting the same
   PAK (instead of silently composing them). Support for registered new
   members through the official encoder still needs a separate
   fully authenticated implementation; its current backend supports
   replacements.
7. Swapped originals have a journal and conditional restore. Optional
   permanent full-game snapshots remain available but disabled by default.

**Open verification:** test mixed original/new-key PAK loading, clean launch
and rollback on the exact Windows game build. Do not infer mixed-key support
from an encoder success code or a prelaunch RSA verification pass.

## Protect the supplied key

- Do not commit the original ZIP, C# source, executable, or private key to the public Git repository; do not echo key contents to logs or reports. Authorized release binaries contain an embedded signing key that recipients can extract.
- The official runtime can be installed by the release or supplied locally. Exact file digests are verified when packaging and again before use. The build runner never executes the signing executable.
- GitHub Actions Secrets protect trusted build/server operations, **not** anything shipped inside a client-side executable. Keys embedded in binaries can be extracted.
- Public verification keys may be bundled; never confuse them with private signing credentials.
- `.gitignore` and the CI credential check help prevent accidents; they do not prove a compiled binary is secret-free.

## Gates before replacing the production backend

- Successful original PAK signature verification and decoding.
- One changed asset rebuilt into the target PAK; all other member contents identical by hash.
- Signed output verifies under the runtime key and actually loads in an **offline** match.
- Two mods in different PAKs rebuild only those PAKs; conflicts are deterministic.
- Exact game-build hook compatibility; no infinite wait/hardcoded-offset corruption after an update.
- Clean game launch remains functional; recovery works after failed, interrupted or killed launches.
- Public Git history and generated diagnostics must contain no raw key material or vendor-source archive. Authorized installers and ZIPs contain the official runtime executable, whose embedded signing material is inherently extractable.

Until these tests pass, retain the current functioning launch/recovery route rather than making an unsafe mixed-key switch.
