# Direct Launch and Client Launch

## Selected launch mode

**Direct Launch is the default**, including for settings files created before the launch mode existed. In **Settings → Game Storage → Set Up Manager**, choose Direct Launch or Client Launch. The preference is saved when selected and is used by both Home Play buttons:

| Action | Direct Launch (default) | Client Launch |
| --- | --- | --- |
| Play without Mods | Verify the game, refuse unresolved original backups, restore normal manager network policy and start `Evolve.exe` as the interactive standard user with original game PAKs | Launch normal `ModdedEvolveLauncher.exe` as the interactive standard user, keeping its repair and profile services |
| Play with Mods | Build/sign enabled projects, prepare and verify the reversible journal, swap custom-signed PAKs and `inject.dll` **before** process creation, arm independent rollback watchdog, then start `Evolve.exe` directly | Keep established Frida `CreateProcessW` interception and client-owned process launch, signed PAK swap, and watchdog |

The Direct Launch button is no longer separate. Both Play buttons use the setting. Switching to Client Launch never erases signing-stage backups or rewrites an active journal.

Direct mode does not require a client executable just to save the game path, build a new signing stage, or run either Play action. Existing Client Launch setups remain supported. Direct startup uses the verified Explorer medium-integrity desktop token (not the manager's elevated administrator token), game binary working directory and the three Steam app-ID environment fields logged by the client. Direct mode uses a separate `direct_standard_user_launch.py` helper rather than overwriting any existing client token broker.

## Recovery

The verified sign/cache preparation runs while game and launcher processes are closed. A direct modded launch revalidates the original/staged file mapping, prepared sizes/digests, and recovery ledger. It records `swapping` **before** any same-volume rename and `swapped` after the final PAK. No launcher pause or Frida hook is needed.

A partial swap attempts immediate recovery through `reusable_restore.py`. After a completed swap, `auto_restore_watch.py` runs in its own detached process and restores originals when the game exits, even if the manager quits. Startup and watchdog failures preserve backups/journal and report recovery actions. Do not launch again if the journal is still active.

## Online/runtime limitation

**Native process creation does not reproduce client services.** In Direct Launch, the current prototype does not implement Modded Evolve local profile/auth endpoints, Stage2Trace game bootstrap, or the full Steam compatibility relay lifecycle. Evolve's process may start without a usable profile or connected services. Successful PID reporting does **not** establish game-playable or online parity. Neither the manager nor this PR modifies the server or claims it can bypass authentication.

Client Launch remains available for cases requiring the existing client runtime. Direct Launch should be validated on an interactive Windows desktop, separately for clean and modded sessions, before releasing to general users.

## Home banner artwork

The supplied `default_loading.dds` is intended as the large Home banner, **not** the application's loading screen. Home reads DXT1/BC1 `assets/default_loading.dds` next to the manager executable (or `default_loading.dds` alongside it), darkens it for readability, and retains usable controls when the file is absent. The DDS is **not yet packaged** in the PR; installer packaging remains a required follow-up before release.

## Verification

- Run Rust/Windows CI, Python direct-launch ordering/regression tests, and install/update tests.
- Launch unmodded Direct Launch with original PAKs; confirm executable working directory, medium integrity and unmodified original files.
- Launch modded Direct Launch with one known-good locally signed PAK; confirm actual game content mounts and all originals return after exit.
- Kill the manager while the game runs; confirm the detached restore watchdog still completes.
- Inject pre-launch swap, game-creation and watchdog failures in a throwaway test installation; inspect recovery logs and verify no original is overwritten or abandoned.
- Compare Stage2Trace, SteamCompat and profile-service initialization with a known working Client Launch session; do not infer network compatibility from process creation alone.
