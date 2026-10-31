# On-demand elevation boundary

Status: a restricted one-shot worker executable has been added with only
`firewall-block`, `firewall-allow`, and read-only `health` commands. The worker
checks the selected game root for links/reparse points, parent traversal and a
regular Evolve.exe before any firewall changes. Windows system tools are
resolved through GetSystemDirectoryW rather than caller-controlled environment
variables. These are **foundations**, not a completed elevation handoff: the
GUI still embeds `requireAdministrator` and never invokes the new worker.
Do not change its manifest until protected actions and recovery use an audited,
tested boundary. This is separate from the UI redesign.

## Contract

- The GUI starts with `asInvoker` and stays at the signed-in user's normal
  integrity level. Reading assets, editing projects in user-writable folders,
  settings, and clean client launch do not ask for UAC.
- On an explicit protected action, request UAC for a short-lived, narrowly
  scoped worker. Cancellation is reported without trying the operation again.
  The worker accepts a closed set of operations, independently validates paths
  and inputs, reports success/failure, and exits. There is no generic command,
  file-copy, or arbitrary Python-script execution operation.
- Protect the whole modded-play transaction, including the manager's outbound
  firewall rule, game-file staging, recovery and watchdog; maintain journal
  ordering and preserve backups on failure. Launch the game/client using the
  existing verified **medium-integrity** token broker, not the worker token.
- Request elevation for the user-selected **System Health** check as requested,
  even though today's `system_health::inspect` only reads state. The worker must
  not acquire broader write authority merely because that check is elevated.
- Only add/remove `EvolveModManager-Offline-*` rules for the selected Evolve
  executable. Never disable Windows Firewall globally or touch unrelated rules.

## Paths to account for

| Path | Current call site | Requirement |
| --- | --- | --- |
| Modded play and offline launch | `main.rs` `PlayEnabledMods`, `StartExistingOfflineLauncher`; `mod_play.rs`; `launcher_bridge.rs` | Single recoverable elevated transaction; verified medium client |
| Offline restore and orphaned backups | `RestoreExistingOfflineLauncher`, `launcher_bridge.rs` | Available without a working client or Frida |
| Online policy and clean/direct play | `online_guard.rs`, `play_setup.rs`, Developer network toggle | UAC for manager-rule mutations, never for a read-only query |
| Signing stage and profile writes | `StageInit/Import/Apply/Restore`, profile operations, initial/test setup | Elevate only if the actual destination is protected |
| Other configured roots and install writes | asset/library/project exports, GFX tool install, updater | User-writable locations stay medium; protected targets need an explicit audited path or fail clearly |
| System Health | `main.rs` `CheckSystemHealth`, `system_health.rs` | Explicit UAC request; report cancelled/failed separately |
| Teleport overlay | second executable in the desktop package | Ensure it does not inherit `requireAdministrator` when GUI changes |

The installer itself may remain admin-only for its Program Files destination.
The desktop test `tests/windows_admin_manifest.rs` and the CI manifest assertion
in `.github/workflows/rust-ci.yml` must change **with** the actual manifest.

## Existing high-integrity hazards to resolve first

`launcher_bridge.rs` executes `py` and some scripts from the user-selected
swap folder. `play_setup.rs` checks the presence of `reusable_restore.py` and
`auto_restore_watch.py`, not their contents. Other scripts are migrated only
against known hashes at setup time, not pinned at execution time. The
interpreter, imports, current directory and files in the swap folder may all
be controlled by a medium-integrity user. Never run these under a new elevated
worker as-is. Use a protected executable/interpreter and authenticated helper
bytes, or move privileged operations into the worker; ensure watchdog recovery
remains possible after worker exit.

The user-writable `controlled_swap_state.json` is not authority to rename an
arbitrary path. Derive game/stage roots and permitted PAK/hook targets inside
the worker; reject path traversal, links/junctions, duplicate targets, forged
hashes and unexpected phases **before** the first write, including recovery.
Do not accept a caller-supplied game root without validating it against the
recorded stage and installed executable. The current manager-owned rule name
is derived from the canonical game executable by `online_guard.rs`.

## Implemented worker foundation and limitations

- `crates/evolve-privileged-worker/src/main.rs` is an independent Cargo-package
  binary with a closed argument parser. It cannot execute caller-supplied
  programs/scripts, copy files, restore backups, or accept a journal path.
  The firewall commands call only `online_guard::apply`, after rejecting
  relative or linked game paths. The health command returns a read-only JSON
  snapshot on stdout.
- The worker currently has **no GUI UAC launcher, authenticated output channel
  or installer packaging contract**. Its separate crate prevents inheriting
  the GUI's `requireAdministrator` manifest through `winres`. The GUI must **not** invoke an arbitrary
  sibling worker from a user-writable portable folder: that would allow
  replacement of the elevated executable. A future broker must verify a
  protected installation and implement an authenticated reply, cancellation,
  process lifetime and status handling. A DLL/resource manifest inspection
  must establish the actual worker and overlay integrity levels before release.
- This worker does not yet offer signing, PAK swap, journal recovery, updater,
  or GFX write operations. Do not enable these through a generic command.
  They require independent path and asset validation before privileged writes.

## Acceptance gates

1. Automated tests reject unknown operations, malformed/oversized requests,
   foreign paths, forged active-phase journals and replaced helpers without
   mutating game files. Test the exact firewall-rule scope and GUI/worker
   manifests on Windows and in CI.
2. Cancel UAC before each protected operation: no file or firewall mutation,
   clear UI feedback, no stuck busy state. Do not retry silently.
3. Kill a modded-play worker during staging and during swap: original files
   remain recoverable; fresh launches refuse an incomplete swap until recovery.
4. Validate ordinary clean play stays medium without UAC; modded play requests
   elevation and launches the actual client at verified medium integrity.
5. Inspect Modern/Legacy GUI as a standard user at 1440×900 and 880×620.

Until these gates pass, a local `asInvoker` build may be used **only** to
review layout; it is not evidence that protected actions work. Never distribute
such a preview as a release.
