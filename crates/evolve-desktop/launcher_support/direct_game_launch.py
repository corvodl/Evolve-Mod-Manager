#!/usr/bin/env python3
"""Launch a verified modded Evolve game directly, without a client process hook.

Private offline testing only. Requires the exact existing prepared swap journal.
The detached existing restore watchdog owns eventual original-file restoration.
Never changes launcher configuration, credentials or remote authentication.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import subprocess
import sys
import json
import time
import uuid

import controlled_swap as swapper
import direct_standard_user_launch as standard_user_launch
import stage2_runtime_contract as contract


def _ordinary(value):
    return Path(swapper.ordinary_windows_path(value)).resolve()


def _regular(path):
    try:
        stat = path.lstat()
    except OSError:
        return False
    import stat as stat_module
    return (stat_module.S_ISREG(stat.st_mode) and
            not (getattr(stat, "st_file_attributes", 0) &
                 getattr(stat_module, "FILE_ATTRIBUTE_REPARSE_POINT", 0)))


def validate_prepared(game_root, stage_root):
    """Fail before any rename unless the staged, signed plan exactly matches."""
    swapper.all_closed()
    state = swapper.require_state()
    if state.get("phase") != "prepared" or state.get("version") != 1:
        raise RuntimeError("Direct launch requires the prepared recovery journal.")
    game, stage = _ordinary(game_root), _ordinary(stage_root)
    if not swapper.same_game_root(game, state.get("game", "")):
        raise RuntimeError("Recovery journal belongs to another game root.")
    if not swapper.same_game_root(stage, state.get("stage", "")):
        raise RuntimeError("Recovery journal belongs to another signing stage.")
    expected, status = swapper.list_files(game, stage)
    files = state.get("files")
    if not isinstance(files, list) or len(files) != len(expected) or state.get("count") != len(expected):
        raise RuntimeError("Prepared recovery journal does not match the stage file count.")
    if len(files) > 513:
        raise RuntimeError("Prepared file count exceeds the supported limit.")
    for given, item in zip(files, expected):
        if not isinstance(given, dict) or given.get("name", "").replace("\\", "/").casefold() != item["name"].replace("\\", "/").casefold():
            raise RuntimeError("Recovery journal file ordering differs from signed stage.")
        for field in ("source", "target", "ready", "backup"):
            if _ordinary(given.get(field, "")) != _ordinary(item[field]):
                raise RuntimeError("Recovery journal path differs from the signed stage: " + field)
        source, target, ready, backup = (Path(given[key]) for key in ("source", "target", "ready", "backup"))
        if not all(_regular(p) for p in (source, target, ready)):
            raise RuntimeError("Original or prepared file is missing / unsafe: " + given["name"])
        if backup.exists() or backup.is_symlink():
            raise RuntimeError("An original backup already exists: " + given["name"])
        if given.get("bytes") != ready.stat().st_size:
            raise RuntimeError("Prepared file size changed: " + given["name"])
        digest = swapper.sha(ready)
        if given.get("sha256") != digest or swapper.sha(source) != digest:
            raise RuntimeError("Prepared file digest differs from signed stage: " + given["name"])
    return state


def rename_prepared_into_game(state):
    """Journal each state transition before moving original/replacement files."""
    swapper.no_game()
    swapper.write_state({**state, "phase": "swapping"})
    try:
        for index, item in enumerate(state["files"], start=1):
            os.replace(item["target"], item["backup"])
            os.replace(item["ready"], item["target"])
            if index % 25 == 0 or index == len(state["files"]):
                print(f"DIRECT_SWAP {index}/{len(state['files'])}", flush=True)
        swapper.write_state({**state, "phase": "swapped"})
    except BaseException:
        swapper.write_state({**state, "phase": "interrupted"})
        raise


def start_restore_watchdog():
    """Run the existing vetted watchdog independently of manager or game PID."""
    script = Path(__file__).with_name("auto_restore_watch.py")
    if not _regular(script):
        raise RuntimeError("Independent restore watchdog is missing.")
    log = script.with_name("rust-direct-restore-watch.log")
    flags = (getattr(subprocess, "DETACHED_PROCESS", 0) |
             getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0) |
             getattr(subprocess, "CREATE_NO_WINDOW", 0))
    with log.open("ab") as stream:
        child = subprocess.Popen(
            ["py", "-3.11", "-u", str(script)],
            cwd=str(script.parent),
            stdin=subprocess.DEVNULL, stdout=stream, stderr=subprocess.STDOUT,
            creationflags=flags, close_fds=True)
    if child.pid <= 0:
        raise RuntimeError("Could not create the independent original-file watchdog.")
    return child.pid


def start_runtime_supervisor(client_exe, game_exe, swap_dir):
    """Start the independent session host before modifying any installed PAK."""
    swap=Path(swap_dir).resolve(strict=True)
    session=swap/("direct-runtime-"+uuid.uuid4().hex)
    session.mkdir(mode=0o700)
    script=Path(__file__).with_name("direct_runtime_supervisor.py")
    if not _regular(script):
        raise RuntimeError("The direct runtime service supervisor was not installed")
    log=swap/"rust-direct-runtime-supervisor.log"
    creationflags=(getattr(subprocess,"DETACHED_PROCESS",0) |
                   getattr(subprocess,"CREATE_NEW_PROCESS_GROUP",0) |
                   getattr(subprocess,"CREATE_NO_WINDOW",0))
    with log.open("ab") as output:
        process=subprocess.Popen(
            [sys.executable,"-u",str(script),"--client",str(client_exe),
             "--game",str(game_exe),"--swap",str(swap),"--session",str(session)],
            cwd=str(swap),stdin=subprocess.DEVNULL,stdout=output,
            stderr=subprocess.STDOUT,close_fds=True,creationflags=creationflags)
    deadline=time.monotonic()+65
    while time.monotonic()<deadline:
        if (session/"error.json").is_file():
            detail=json.loads((session/"error.json").read_text())
            raise RuntimeError("SteamCompat/profile service startup failed: "+str(detail.get("error","unknown")))
        if (session/"ready.json").is_file():
            state=json.loads((session/"ready.json").read_text())
            if state.get("service_pid",0)>0 and all(
                Path(state[field]).is_file()
                for field in ("trace","steam_compat")
            ) and state.get("modes"):
                return session,state
            raise RuntimeError("Runtime host returned incomplete compatibility initialization")
        if process.poll() is not None:
            raise RuntimeError("Runtime service supervisor exited before the game was ready")
        time.sleep(.25)
    (session/"cancel").write_text("cancel")
    raise RuntimeError("Runtime profile/Steam compatibility initialization timed out; PAK files were not swapped")


def stop_runtime_before_restore(session, timeout=30):
    """Ask the private service host to exit before the normal restore preflight.

    The original-file helper refuses to work while ModdedEvolveLauncher.exe
    exists, including our own headless service host. Do not force a restore
    while it is still alive.
    """
    if session is None:
        return
    (session/"cancel").write_text("cancel", encoding="utf-8")
    deadline=time.monotonic()+timeout
    while time.monotonic()<deadline:
        if (session/"stopped.json").is_file():
            return
        time.sleep(0.2)
    raise RuntimeError(
        "Client service host did not stop. Original PAK backups and journal "
        "are preserved. Close the host and use Backups & Recovery before relaunch."
    )


def environment_for_session(state, swap):
    # Every required path, hash and mode is verified here. In particular, do
    # not make up missing relay or identity values: the client must supply
    # them through the services supervisor when that interface is available.
    return contract.build_game_environment(state, swap)


def has_visible_game_window(pid):
    """Check for a real top-level window owned by this game PID, not a hook log."""
    if os.name != "nt":
        return False
    import ctypes
    from ctypes import wintypes

    user32 = ctypes.WinDLL("user32", use_last_error=True)
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
    user32.EnumWindows.restype = wintypes.BOOL
    user32.GetWindowThreadProcessId.argtypes = [
        wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.GetWindowThreadProcessId.restype = wintypes.DWORD
    user32.IsWindowVisible.argtypes = [wintypes.HWND]
    user32.IsWindowVisible.restype = wintypes.BOOL
    found = []

    @callback_type
    def check(hwnd, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user32.IsWindowVisible(hwnd):
            found.append(hwnd)
            return False
        return True

    result = user32.EnumWindows(check, 0)
    if not result and not found:
        raise RuntimeError(
            "Cannot enumerate game windows (Windows error "
            + str(ctypes.get_last_error()) + ")")
    return bool(found)


def wait_for_visible_game_window(process, timeout=45, stable_seconds=5):
    """Do not claim success until Evolve has a visible, stable game window.

    Loading Stage2Trace/SteamCompat only proves a suspended game process was
    resumed. A process that exits a few seconds later is a failed launch.
    """
    deadline = time.monotonic() + timeout
    visible_since = None
    while time.monotonic() < deadline:
        code = process.poll()
        if code is not None:
            raise RuntimeError(
                f"Evolve.exe exited before a stable visible window "
                f"(PID {process.pid}, exit code {code}). "
                "Inspect rust-direct-stage2-hook.log and the game's crash logs.")
        now = time.monotonic()
        if has_visible_game_window(process.pid):
            if visible_since is None:
                visible_since = now
            if now - visible_since >= stable_seconds:
                return
        else:
            visible_since = None
        time.sleep(0.2)
    raise RuntimeError(
        f"Evolve.exe PID {process.pid} never displayed a stable visible "
        f"game window within {timeout} seconds. Check the game's crash logs.")


def launch_direct(game_root, stage_root=None, client_exe=None, swap_dir=None, clean=False):
    game=_ordinary(game_root)
    exe=game/"bin64_SteamRetail"/"Evolve.exe"
    if not _regular(exe):
        raise RuntimeError("Installed Evolve.exe was not found")
    if not client_exe or not swap_dir:
        raise RuntimeError("Direct Launch requires the original client runtime to host profile and Steam services")
    swap=Path(swap_dir).resolve(strict=True)
    session=None
    watchdog=None
    swapped=False
    swap_attempted=False
    # This preflight requires ALL client/game processes closed; run it BEFORE
    # our own Stage2 profile service host becomes active. Validation doesn't
    # change game files. The actual swap still happens after services are ready.
    prepared = validate_prepared(game,stage_root) if not clean else None
    try:
        # Run services/identity/profile bootstrap while originals are installed.
        # Only a successful services host may proceed to the PAK swap.
        session,state=start_runtime_supervisor(client_exe,exe,swap)
        # Validate the exact October 15 Stage2Trace build, compatible
        # game-local Steam provider and *file* catalog before any PAK rename.
        runtime_env=environment_for_session(state,swap)
        hook_log=Path(runtime_env["MODDED_EVOLVE_STAGE2_LOG"])
        if hook_log.exists() and not _regular(hook_log):
            raise RuntimeError("Stage2 hook log is not a regular file; refusing startup")
        # Truncate the manager-owned log before starting the suspended game:
        # successes from a previous session must never pass new startup checks.
        hook_log.write_text("", encoding="utf-8")
        if not clean:
            swap_attempted=True
            rename_prepared_into_game(prepared)
            swapped=True
            watchdog=start_restore_watchdog()
        try:
            process=standard_user_launch.launch_standard_user(
                exe,game=True,trace_dll=state["trace"],
                runtime_env=runtime_env)
            try:
                try:
                    contract.wait_for_hook_startup(
                        hook_log, Path(state["steam_compat"]), state["modes"], process
                    )
                    wait_for_visible_game_window(process)
                except BaseException:
                    if process.poll() is None:
                        process.terminate()
                    raise
                (session/"game.pid").write_text(str(process.pid)+"\n")
                print(f"DIRECT_GAME_LAUNCHED pid={process.pid} "
                      f"watchdog={watchdog or 0} swap_complete={str(swapped).lower()} "
                      "services_ready=true trace_loaded=true hooks_ready=true "
                      "window_visible=true",flush=True)
                return 0
            finally:
                process.close()
        except BaseException as error:
            raise RuntimeError(
                f"Suspended game startup failed ({error}); Stage2 services will stop. "
                "Original PAK restoration remains owned by the detached watchdog."
            ) from error
    except BaseException as error:
        # The session host has the same process name as the normal client, so
        # even a partial rename cannot be restored until it has really exited.
        if session is not None:
            stop_runtime_before_restore(session)
        if swap_attempted and watchdog is None:
            # This also covers a rename failure that left the journal in
            # 'interrupted' and one or more original files in backup slots.
            # The tested restore helper remains the single owner of renames.
            from reusable_restore import main as restore
            if restore()!=0:
                raise RuntimeError(
                    f"Direct launch failed ({error}) and automatic PAK recovery failed. "
                    "Keep every backup and the journal, then use Backups & Recovery."
                ) from error
        raise


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game-root",required=True)
    parser.add_argument("--stage-dir")
    parser.add_argument("--client-exe",required=True)
    parser.add_argument("--swap-dir",required=True)
    parser.add_argument("--clean",action="store_true")
    args=parser.parse_args(argv)
    try:
        if not args.clean and not args.stage_dir:
            raise RuntimeError("A verified signing stage is required for modded direct play")
        return launch_direct(args.game_root,args.stage_dir,args.client_exe,args.swap_dir,args.clean)
    except BaseException as error:
        print(f"DIRECT_LAUNCH_FAILED: {error}",file=sys.stderr,flush=True)
        return 1


if __name__=="__main__":
    sys.exit(main())
