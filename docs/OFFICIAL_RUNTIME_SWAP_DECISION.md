# Stage 2 runtime file replacement decision

The developer-supplied May 2025 archive contains three executable commands:

| Command | Purpose |
| --- | --- |
| `d <pak> <output-directory>` | Decode one PAK into an extracted directory |
| `e <input-directory> <pak>` | Encode one complete directory as a PAK |
| `r <game-directory>` | Re-sign PAK files recursively under the selected directory |

Its `howto.txt` directs modders to run `r` and then manually copy `inject.dll` and `dbghelp.dll` to `Bin64_SteamRetail`. The included `decode.cmd`, `encode.cmd`, and `repack.cmd` are command wrappers for those three operations. No launcher interception, reversible live swapping, recovery journal, or watchdog is supplied.

**Production decision:** integrate the official signing and verification components, but retain the manager's reversible launch-time swap and restore helpers. The supplied manual replacement instructions do not implement equivalent automated restoration or interrupted-launch recovery.

**Performance:** do the full stock archive key conversion **once per installed game build**, in isolated storage. Only edited PAKs are re-encoded when mods change. Cache unchanged outputs. For a clean launch, original PAKs and DLLs must remain recoverable. The launch-time replacement set can be optimized independently, but a new-key-only verifier may still require the entire converted set while modded.

**Not measured:** the relative game launch latency of manual persistent installation versus the manager's cached, reversible swap. A persistent install could avoid repeated swaps, but it changes the installed game's normal state until the originals are restored. Do not silently switch to persistent installation merely because the official manual instructions use file copying.

**Validation:** verify an official-key PAK loads in an offline match, the correct DLLs load for the game build, the restore watchdog recovers all originals, and clean launch still works. Preserve diagnostic details without including credential material.
