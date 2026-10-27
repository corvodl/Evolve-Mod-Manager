# Official Stage 2 developer toolchain: integration assessment

> Source: developer-provided `evolve_tool_stage2_02.05.2025.zip`. Static inspection only. Neither the Windows binaries nor game runtime were executed. This does **not** change the current launch pipeline or establish redistribution permission.

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
