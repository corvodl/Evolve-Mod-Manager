# Windows process elevation separation

The Evolve Mod Manager installer and GUI intentionally run as administrator, because the manager can prepare/recover protected game PAK files and configure a manager-owned Windows Firewall rule. **The Modded Evolve client and Evolve.exe should instead run under the signed-in user's medium-integrity token.**

## Implementation

The Rust manager launches the normal client using the new `standard_user_launch.py` broker rather than inheriting its elevated token. The broker uses the *current desktop* Explorer window, verifies that its owner is `explorer.exe`, checks that the shell belongs to the manager's Windows session and that its token is non-elevated at integrity level 0x2000 (Medium), duplicates that primary token, and starts `ModdedEvolveLauncher.exe` with `CreateProcessWithTokenW` and a user environment block. It queries the actual resulting process token before reporting success. There is **no elevated subprocess fallback**. It never asks for or stores credentials and never modifies remote authentication.

Play without Mods uses the same broker. The existing Python 3.11 x64 runtime is therefore required for both launch paths. If Windows does not expose an accessible, non-elevated Explorer shell in the same desktop session, launch returns an actionable error.

Play with Mods preserves the previous privilege boundary: the Rust manager and Python/Frida helper remain elevated for safe, reversible, same-volume PAK swaps. The verified standard-user Modded Evolve launcher starts `Evolve.exe`. The Frida `CreateProcessW` hook waits until **after** `CreateProcessW` returns and checks the actual Evolve PID for a non-elevated medium-integrity process before reporting game release. The separate restore watchdog is retained. If game creation fails or returns an elevated process, the launch is reported as failed and the existing recovery watchdog is still armed when the swap occurred.

The standard-user script is created only if missing and rejects an existing unrecognized helper file; the previous Nightly .69 `auto_launch_frida.py` digest is explicitly accepted for safe migration.

## Diagnostics

In the unlocked Developer tab, click **Inspect Client / Game Privileges** while the client/game are running. The read-only diagnostic reports process names, PIDs, Windows logon sessions and actual elevated/medium/high status for the manager, Modded Evolve launcher, and Evolve.exe. The modded launch log `rust-launch-bridge.log` also captures the helper/client/game elevation checks.

Expected:

| Process | Token |
| --- | --- |
| Evolve Mod Manager | High (Administrator) |
| Python/Frida swap helper | High (Administrator) |
| ModdedEvolveLauncher.exe | Medium (Standard user) |
| Evolve.exe | Medium (Standard user) |

Do not confuse successful Windows process creation with successful profile-service authentication. The clean client already connected before this change despite apparently inheriting elevation, so the root cause of profile failures during modified-PAK launches is **not established** by privilege checks alone. Compare the original and modded game with the same launcher, client version, profile service and firewall mode.

## Verification

Static Python tests and Rust tests run in CI. GitHub Windows runner jobs are headless: they cannot prove that the normal interactive Explorer token/Frida launch works, so **test this change in a user desktop session before enabling a public default**. Restore all original game assets on any failed modded launch before starting another test.
