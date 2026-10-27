# Experimental manager-owned Evolve launch and Home artwork

## Home artwork
The supplied `default_loading.dds` is **Home artwork**, not the manager's loading screen. The Home page looks for `assets/default_loading.dds` next to the installed manager executable, then `default_loading.dds` next to the executable. It accepts bounded DXT1 / BC1 DDS images and darkens RGB to 52% brightness so controls remain readable. The 1920x1080 source should be distributed as `assets/default_loading.dds` in a follow-up packaging commit. The art is not embedded in this PR because this change only ships the reader. Without the file, Home remains usable and unadorned.

## Direct launch
The **Direct Offline Launch · Experimental** Home action starts installed `bin64_SteamRetail/Evolve.exe` under the verified interactive medium-integrity Explorer token rather than inheriting the elevated manager token. The working directory is the game binary directory. The three Steam app ID fields observed in the client launch logs are set to 273350. No foreign launcher binary, local profile service, identity bridge or networking impersonation is started. Manager-owned offline firewall controls remain enabled for this test. Direct launch refuses unresolved recovery warnings and orphaned backups.

This does **not** reimplement the client runtime, profile APIs, DLL initialization, Stage2Trace, SteamCompat relay, or automatic mod swapping. Therefore it is not a replacement for Play with Mods / Play without Mods yet. Both existing client-backed flows remain unchanged. Direct launch is intended only for diagnostic offline startup comparisons.

## Verification required before defaulting
- On a real Windows desktop, ensure direct process has the same medium-integrity token and expected game working directory; test without mods.
- Confirm start/exit works, and identify missing client profile, runtime and networking services.
- Confirm original game files are unchanged after direct launch, including failed starts.
- Verify an actual direct-client replacement separately before using online features.
- Package the user's exact DDS in the installer and confirm banner rendering; this PR only implements the loader and layout.
