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

import controlled_swap as swapper
import direct_standard_user_launch as standard_user_launch


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


def launch_direct(game_root, stage_root):
    game = _ordinary(game_root)
    stage = _ordinary(stage_root)
    exe = game / "bin64_SteamRetail" / "Evolve.exe"
    if not _regular(exe):
        raise RuntimeError("Installed Evolve.exe was not found.")
    state = validate_prepared(game, stage)
    try:
        rename_prepared_into_game(state)
    except Exception as swap_error:
        # Evolve.exe does not exist yet. Immediately attempt the vetted
        # original-file recovery; never discard journal/backups on failure.
        from reusable_restore import main as restore
        if restore() != 0:
            raise RuntimeError(
                f"Pre-launch swap failed ({swap_error}); automatic restore also failed. "
                "Do not relaunch until originals are recovered.") from swap_error
        raise RuntimeError(
            f"Pre-launch swap failed ({swap_error}); originals restored.") from swap_error
    try:
        watch_pid = start_restore_watchdog()
    except Exception as problem:
        # No game exists yet; best-effort immediate rollback through the
        # existing verified restoration helper, never delete backups.
        try:
            from reusable_restore import main as restore
            if restore() != 0:
                raise RuntimeError("Restoration helper reported an error.")
        except Exception as recovery:
            raise RuntimeError(
                f"Watchdog unavailable ({problem}); recovery failed ({recovery}). "
                "Do not launch again until originals are restored.") from problem
        raise RuntimeError(f"Watchdog unavailable ({problem}); originals restored.") from problem

    try:
        process = standard_user_launch.launch_standard_user(exe, game=True)
        try:
            print(f"DIRECT_GAME_LAUNCHED pid={process.pid} watchdog={watch_pid} swap_complete=true",
                  flush=True)
            return 0
        finally:
            process.close()  # Closes only our handle; game continues running.
    except Exception as error:
        # Watchdog remains independently alive and restores when all game and
        # launcher processes are gone, even if the manager itself exits.
        raise RuntimeError(
            f"Game creation failed ({error}); the active restore watchdog "
            f"(PID {watch_pid}) will restore originals. Do not relaunch until restored.") from error


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game-root", required=True)
    parser.add_argument("--stage-dir", required=True)
    args = parser.parse_args(argv)
    try:
        return launch_direct(args.game_root, args.stage_dir)
    except Exception as error:
        print(f"DIRECT_LAUNCH_FAILED: {error}", file=sys.stderr, flush=True)
        return 1


if __name__ == "__main__":
    sys.exit(main())
