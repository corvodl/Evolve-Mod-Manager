#!/usr/bin/env python3
"""Preserve stale *prepared* private-helper copies before preparing a new stage.

This tool never changes an installed PAK or injector. It is for a clean
``prepared`` journal only, after an independently recoverable mod profile was
applied to the signing stage. It preserves the old ready copies and journal
before asking the existing private helper to prepare the new ones. Do not use
for active/interrupted swaps; use the manager's normal restore workflow there.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import uuid

MAX_PAKS = 512
MAX_FILE = 16 * 1024**3
BACKUP = ".customkey-original"
READY = ".customkey-ready"
HEX = re.compile(r"[0-9a-f]{64}\Z")


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(128 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def regular(path: Path) -> bool:
    try:
        info = path.lstat()
    except FileNotFoundError:
        return False
    return (stat.S_ISREG(info.st_mode)
            and not (getattr(info, "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT)
            and 0 < info.st_size <= MAX_FILE)


def checked_json(path: Path) -> dict:
    if not regular(path) or path.stat().st_size > 4 * 1024 * 1024:
        raise ValueError(f"Missing or unsafe recovery document: {path}")
    value = json.loads(path.read_bytes())
    if not isinstance(value, dict):
        raise ValueError("Recovery document must be an object")
    return value


def relative(raw: str) -> Path:
    normalized = raw.replace("\\", "/")
    parts = normalized.split("/")
    if (not normalized or len(normalized) > 600 or ":" in normalized
            or any(part in ("", ".", "..") for part in parts)
            or not normalized.lower().endswith(".pak")):
        raise ValueError(f"Unsafe signed PAK path: {raw}")
    return Path(*parts)


def no_games() -> None:
    if os.name != "nt":
        return
    result = subprocess.run(["tasklist", "/FO", "CSV", "/NH"],
                            capture_output=True, check=True, text=True)
    for line in result.stdout.lower().splitlines():
        if line.startswith(('"evolve.exe",', '"moddedevolvelauncher.exe",')):
            raise ValueError("Close Evolve and ModdedEvolveLauncher before refreshing")


def same(left: Path, right: Path) -> bool:
    return left.samefile(right)


def safe_parent(root: Path, child: Path) -> bool:
    try:
        root = root.resolve(strict=True)
        child = child.resolve(strict=True)
        if not child.is_relative_to(root):
            return False
        while child != root:
            info = child.lstat()
            if not stat.S_ISDIR(info.st_mode) or (getattr(info, "st_file_attributes", 0)
                    & stat.FILE_ATTRIBUTE_REPARSE_POINT):
                return False
            child = child.parent
        return True
    except OSError:
        return False


def ordinary_windows(raw: str) -> str:
    # Python keeps the extended-length prefix on resolve(), whereas the
    # private journal intentionally records ordinary Win32 paths.
    if raw.startswith("\\\\?\\UNC\\"):
        return "\\\\" + raw[8:]
    if raw.startswith("\\\\?\\"):
        return raw[4:]
    return raw


def plan_refresh(stage: Path, swap: Path) -> tuple[dict, list[tuple[Path, Path, Path, str]]]:
    stage = stage.resolve(strict=True)
    swap = swap.resolve(strict=True)
    plan = checked_json(stage / "rekey_plan.json")
    status = checked_json(stage / "stage_status.json")
    journal = checked_json(swap / "controlled_swap_state.json")
    if (plan.get("blocked") or status.get("status") != "fully-staged-offline"
            or journal.get("version") != 1 or journal.get("phase") != "prepared"):
        raise ValueError("Only an unblocked, fully staged, clean prepared journal can refresh")
    game = Path(ordinary_windows(plan["source_root"])).resolve(strict=True)
    if (not game.is_dir() or stage == game or swap == game
            or stage in game.parents or game in stage.parents
            or swap in game.parents or game in swap.parents
            or not same(Path(journal["game"]), game)
            or not same(Path(journal["stage"]), stage)):
        raise ValueError("Journal/stage/installed game roots do not agree")
    entries = [e for e in plan["entries"] if e.get("kind") == "signed-encrypted"]
    records = journal.get("files")
    if (not entries or len(entries) > MAX_PAKS or not isinstance(records, list)
            or len(records) != len(entries) + 1 or journal.get("count") != len(records)
            or status.get("count") != len(entries)):
        raise ValueError("Prepared journal count differs from signed stage plan")
    if any(path.name.lower().endswith(BACKUP) for path in game.rglob("*")):
        raise ValueError("An original game backup exists; use recovery, not prepared refresh")
    seen = set()
    changes = []
    for i, entry in enumerate(entries):
        rel = relative(entry["path"])
        key = str(rel).casefold()
        if key in seen:
            raise ValueError("Duplicate signed destination")
        seen.add(key)
        name = rel.name
        signed = entry.get("signed_name", name)
        if signed not in (name, name.lower()):
            raise ValueError("Invalid signed PAK basename")
        target = game / rel
        source = stage / "paks" / rel
        if not safe_parent(game, target.parent) or not safe_parent(stage, source.parent):
            raise ValueError(f"Prepared target/source escapes its verified root: {rel}")
        record = records[i]
        ready = Path(str(target) + READY)
        backup = Path(str(target) + BACKUP)
        if (str(record["name"]).replace("\\", "/").casefold() != rel.as_posix().casefold()
                or not same(Path(record["target"]), target)
                or not same(Path(record["source"]), source)
                or Path(record["ready"]) != ready
                or Path(record["backup"]) != backup
                or backup.exists() or backup.is_symlink()
                or not regular(target) or not regular(source)
                or not HEX.fullmatch(str(record.get("sha256", "")))):
            raise ValueError(f"Prepared record changed or unsafe: {rel}")
        old_hash = record["sha256"]
        new_hash = digest(source)
        preserved = Path(str(ready) + ".stale-" + old_hash[:16])
        if new_hash != old_hash:
            if regular(ready):
                if preserved.exists() or preserved.is_symlink() or digest(ready) != old_hash:
                    raise ValueError(f"Unexpected existing prepared copy or stale destination: {rel}")
                changes.append((ready, preserved, source, signed))
            elif regular(preserved) and digest(preserved) == old_hash:
                # A previous attempt renamed the ready file before the journal
                # update; resume only the same exact old SHA-256.
                changes.append((ready, preserved, source, signed))
            else:
                raise ValueError(f"Missing/changed prepared copy without a verified saved copy: {rel}")
        elif not regular(ready) or digest(ready) != old_hash:
            raise ValueError(f"Unchanged prepared copy differs: {rel}")
    # The injector is always last, with the exact installed original and
    # pinned staged custom shim. It must NOT be part of a PAK refresh.
    shim = records[-1]
    hook = game / "bin64_SteamRetail" / "inject.dll"
    custom = stage / "inject-custom.dll"
    if (str(shim["name"]).replace("\\", "/").casefold() != "bin64_steamretail/inject.dll"
            or not same(Path(shim["target"]), hook)
            or not same(Path(shim["source"]), custom)
            or Path(shim["ready"]) != Path(str(hook) + READY)
            or Path(shim["backup"]) != Path(str(hook) + BACKUP)
            or not regular(hook) or not regular(custom)
            or not regular(Path(shim["ready"]))
            or digest(Path(shim["ready"])) != shim["sha256"]
            or digest(custom) != shim["sha256"]):
        raise ValueError("Prepared injector mismatch; do not refresh")
    if not changes:
        raise ValueError("No changed staged sources; the prepared journal requires no refresh")
    return journal, changes


def refresh(stage: Path, swap: Path, cli: Path, apply: bool) -> None:
    no_games()
    journal, changes = plan_refresh(stage, swap)
    key = stage / "mykeys" / "public_key.bin"
    if not regular(cli) or not regular(key):
        raise ValueError("Explicit signed-PAK inspector and staged public key are required")
    for ready, preserved, source, signed in changes:
        result = subprocess.run([str(cli), "signed", "inspect", str(source), str(key), signed],
                                capture_output=True, text=True)
        if result.returncode:
            raise ValueError(f"Staged source did not authenticate: {source.name}: {result.stderr.strip()}")
    print("Clean prepared journal; no game backups. Changed staged PAKs:")
    for ready, preserved, source, _ in changes:
        print(f"  {source.name}: preserve {ready.name} -> {preserved.name}")
    if not apply:
        print("Read-only preflight passed. Nothing changed.")
        return
    no_games()
    journal_path = swap / "controlled_swap_state.json"
    copy_path = swap / f"controlled_swap_state.json.pre-refresh-{uuid.uuid4().hex}"
    with copy_path.open("xb") as output, journal_path.open("rb") as source:
        shutil.copyfileobj(source, output)
        output.flush()
        os.fsync(output.fileno())
    if checked_json(copy_path) != journal:
        raise ValueError("Preserved recovery journal differs; no ready copy moved")
    for ready, preserved, _, _ in changes:
        no_games()
        if regular(ready):
            if preserved.exists() or preserved.is_symlink():
                raise ValueError("Stale destination appeared; journal preserved, no overwrite")
            ready.rename(preserved)
    # The helper accepts a restored journal with no backups and rechecks every
    # remaining ready copy before reusing it. Atomic publication leaves either
    # the old 'prepared' journal or the new 'restored' one after a crash.
    journal["phase"] = "restored"
    staging = swap / f".prepared-refresh-{uuid.uuid4().hex}.json"
    with staging.open("x", encoding="utf-8") as output:
        json.dump(journal, output, indent=2)
        output.write("\n")
        output.flush()
        os.fsync(output.fileno())
    no_games()
    os.replace(staging, journal_path)
    print("Old journal and prepared copies preserved; journal is restored/no backups.")
    print("Run the existing private helper's prepare action next; DO NOT launch until manager verification passes.")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", required=True, type=Path)
    parser.add_argument("--swap", required=True, type=Path)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--apply", action="store_true", help="explicitly preserve stale ready files and journal")
    args = parser.parse_args()
    try:
        refresh(args.stage, args.swap, args.cli, args.apply)
        return 0
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(f"Prepared refresh stopped: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
