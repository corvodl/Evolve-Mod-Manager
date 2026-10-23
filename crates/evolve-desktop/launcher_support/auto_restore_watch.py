#!/usr/bin/env python3
"""Out-of-process restoration watchdog for a manager-owned modded launch.

Runs independently of the Qt manager so closing the manager cannot strand a
successful game session with temporary PAK swaps. Only the existing verified
swap journal and reusable_restore are permitted to modify game files.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import sys
import time

import controlled_swap as swap

ACTIVE = frozenset(("evolve.exe", "moddedevolvelauncher.exe"))


def _lock():
    """Only one process may restore this swap at a time."""
    import contextlib
    @contextlib.contextmanager
    def guard():
        path = Path(__file__).with_name("auto_restore_watch.lock")
        with path.open("a+b") as stream:
            # Never write through another watchdog's locked byte.
            if stream.seek(0, 2) == 0:
                stream.write(b"0")
                stream.flush()
            stream.seek(0)
            try:
                if os.name == "nt":
                    import msvcrt
                    msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
                else:
                    import fcntl
                    fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            except OSError:
                yield False
                return
            try:
                yield True
            finally:
                if os.name == "nt":
                    stream.seek(0)
                    msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)
                else:
                    import fcntl
                    fcntl.flock(stream.fileno(), fcntl.LOCK_UN)
    return guard()


def should_restore(names, game_seen, quiet_samples, fresh, elapsed):
    """A fresh launch must first see Evolve, unless launch failed completely.

    A fresh game launch gets a two-minute grace period if Evolve never appears;
    recovery mode trusts four consecutive empty checks after manager restart.
    """
    if ACTIVE.intersection(names):
        return False
    if quiet_samples < 4:
        return False
    return game_seen or not fresh or elapsed >= 120


def watch(fresh=True, interval=2.0, monotonic=time.monotonic, sleeper=time.sleep):
    """Wait until the entire game/client session is gone; never force-close it."""
    from reusable_restore import main as restore_main

    begin = monotonic()
    game_seen = False
    quiet = 0
    while True:
        state = swap.require_state()
        phase = state.get("phase")
        if phase in ("prepared", "restored"):
            if any(Path(item["backup"]).exists()
                   for item in state.get("files", ())):
                raise RuntimeError("Original backup remains despite a clean journal.")
            print("Original files are already restored.", flush=True)
            return 0
        if phase not in ("swapped", "interrupted", "restoring"):
            raise RuntimeError("Automatic restoration refused unsafe phase: " + str(phase))
        names = swap.task_names()
        if "evolve.exe" in names:
            game_seen = True
        quiet = 0 if ACTIVE.intersection(names) else quiet + 1
        if should_restore(names, game_seen, quiet, fresh, monotonic() - begin):
            print("Game and launcher exited. Restoring verified originals...", flush=True)
            # reusable_restore performs its OWN tasklist preflight, checksum
            # validation and atomic rename/recovery steps.
            result = restore_main()
            if result == 0:
                print("Automatic restoration completed.", flush=True)
            return result
        sleeper(interval)


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("--resume", action="store_true",
                        help="Recover a prior session after manager restart.")
    args = parser.parse_args(argv)
    try:
        with _lock() as acquired:
            if not acquired:
                print("Another auto-restore watchdog is already active.", flush=True)
                return 0
            return watch(fresh=not args.resume)
    except Exception as error:
        print("AUTO-RESTORE NEEDS ATTENTION:", error, file=sys.stderr, flush=True)
        print("Backups and journal have NOT been discarded. Use Settings > "
              "Backups & Recovery once game and client close.", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
