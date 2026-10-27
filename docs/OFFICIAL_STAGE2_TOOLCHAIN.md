# Official Stage 2 developer toolchain: integration assessment

> Source: developer-provided `evolve_tool_stage2_02.05.2025.zip`. Static inspection only. Neither the Windows binaries nor game runtime were executed. This does **not** change the current launch pipeline or establish redistribution permission.

## Implementation (PR #101)

The desktop manager now supports the developer's official **2025** encoder as an
explicitly local, pinned toolchain.

1. Extract the developer ZIP privately. Put its \`evolve_tool_stage2\` folder at
   \`<Mod Data Location>/official-stage2-tools/evolve_tool_stage2\`, with the
   original \`evolve_tool.exe\`, \`crypto.dll\`, and \`inject/\` directory inside.
   Alternatively, set \`EVOLVE_STAGE2_TOOLS_DIR\` to the directory containing
   \`evolve_tool.exe\`.
2. Keep the original game/client installation intact. In particular, the
   official tools currently require an existing original
   \`bin64_SteamRetail/dbghelp.dll\` so a complete, reversible swap is possible.
3. Choose **Set Up Manager** with the game and storage root. It detects and
   checksum-verifies the installed local developer tools, creates the public
   key stage, copies original signed PAKs into **isolated** storage and runs
   the official \`r\` conversion there **once**. It authenticates all resulting
   signed PAKs against the developer's public key before marking setup ready.
4. Import/edit a PAK through the normal Modding page. **Build** uses the
   official \`d\` and \`e\` commands for only the PAK containing edited assets,
   verifies original member inventory, validates the signed result, and
   reuses a SHA-keyed cache when the mod has not changed.
5. **Play with Mods** stages only the enabled project's changed PAK outputs,
   preserving the offline recovery journal. The launch helper continues to
   swap the **one-time converted baseline** plus both official runtime DLLs
   because the supplied RSA hook has only one active verification key.

This changes the per-mod build cost but does **not** yet establish that
unconverted stock-key PAKs can be mounted alongside official-new-key PAKs.
Switching to a sparse *runtime* swap is not safe until verified in a live game.
The developer's own instructions explicitly call for a one-time conversion of
the full PAK library.

**Never** run the supplied \`r\` directly against your only installed copy of
the game; the manager only runs it against isolated copies. Do not commit
the supplied ZIP, DLLs, executable, embedded private key or extracted source.

### Validation not yet performed

Windows build/CI checks are required. The provided binaries have not been
executed in this environment, and in-game acceptance, alternative engine builds
and the DLL compatibility offset need testing before a release. The tool may
return success even if a particular PAK failed; the integration therefore
requires independent RSA verification and preserves failures for inspection.

## Verified capabilities and constraints

- `evolve_tool.exe d <pak> <dir>` decrypts and unpacks a PAK; `evolve_tool.exe e <dir> <pak>` packs a **complete** directory as one PAK. These are not in-place single-member editing commands.
- `evolve_tool.exe r <game dir>` traverses the existing PAK library and changes each archive's authenticated trailer to use the **new** RSA keypair, without necessarily rewriting encrypted member payloads.
- The instructions require the supplied `inject.dll` and `dbghelp.dll` in `Bin64_SteamRetail`. The inject assembly has a game-executable-specific hook offset that needs validation for each supported build.
- The C# source contains an embedded **RSA private key**; it is also relied upon by the executable. **Never check in, print, or bundle the source/executable/archive into public releases without a separate authorization and key security decision.**
- The original Stage 2 verification key in this source is **public**, not the original private signing key. A PAK signed with the new private key cannot be assumed compatible with the unmodified stock verifier. Mixing stock old-key archives and new-key mod archives is unverified.

## Goal: only process PAKs containing edited assets

1. Verify the installed game's original PAK hashes, file ownership, public key and binary/hook compatibility. Preserve an untouched clean backup and recovery journal.
2. If the runtime only supports the new verifier, perform the stock archive key transition **once per exact game build**. Persist and reuse the verified converted baseline; revalidate after a game update.
3. Map changed mod members to the PAKs that contain them. Detect conflicting edits and compute a deterministic effective member set.
4. Rebuild **only affected PAKs**. Because official `e` encodes a whole directory, any official-backend integration must first preserve all unmodified members of that PAK, not create an incomplete delta PAK. The current Rust builder can already replace individual members; compare both backends' authenticated outputs before switching the production implementation.
5. Cache the replacement PAK by stock digest, selected changed member digests, active public/signing-key identity, and tool version. Rebuilding or re-signing unrelated PAKs after every mod change is unnecessary.
6. Reduce the **runtime swap list** separately: a single new-key verifier may still require mounting the once-converted stock set even though only one mod PAK is new. A sparse swap with unconverted original PAKs must not be enabled without in-game proof that the verifier supports both keys or that supported overlays work.
7. Restore the original key configuration and all original assets for an explicitly clean launch; verify interrupted and crash recovery.

## Protect the supplied key

- Do not commit the original ZIP, its C# source, the executable, or private key, or write any key bytes to PR discussions, logs, crash reports, support bundles, or release assets.
- Keep the official tool outside the repository as a user-supplied local integration. Use exact tool-version/architecture verification before invoking it; never execute the downloaded executable in CI solely to probe it.
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
- Final public installer, ZIP, repo and generated diagnostics contain **no private key or official tool binaries**.

Until these tests pass, retain the current functioning launch/recovery route rather than making an unsafe mixed-key switch.
