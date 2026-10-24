#!/usr/bin/env python3
"""Restore interrupted Evolve swaps even when the swap-state journal is missing.

Original .customkey-original files are only renamed, never removed. Existing
live files are retained under unique .customkey-recovery-mod-* names. A
durable per-file recovery manifest is stored outside the game directory.
"""
from __future__ import annotations

import argparse
import csv
from dataclasses import dataclass
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import subprocess
import sys
import uuid

SUFFIX = '.customkey-original'


@dataclass(frozen=True)
class RecoveryItem:
    backup: Path
    target: Path
    live_exists: bool
    bytes: int


def _require_safe_file(game: Path, path: Path) -> None:
    """Reject symlinks, junction escapes and unexpected source locations."""
    if not path.is_file() or path.is_symlink():
        raise ValueError('Missing or linked game file; inspect manually: ' + str(path))
    relative = path.relative_to(game)
    cursor = game
    for part in relative.parts[:-1]:
        cursor = cursor / part
        if cursor.is_symlink() or not cursor.is_dir():
            raise ValueError('Linked or missing parent folder: ' + str(cursor))
    if not path.resolve(strict=True).is_relative_to(game):
        raise ValueError('Game file escapes the selected installation: ' + str(path))


def plan_recovery(game: Path) -> list[RecoveryItem]:
    """Read-only inventory. No journal is needed, but the game root is checked."""
    game = Path(game).resolve(strict=True)
    if not game.is_dir() or not (game / 'bin64_SteamRetail' / 'Evolve.exe').is_file():
        raise ValueError('Select the installed EvolveGame folder containing bin64_SteamRetail/Evolve.exe.')
    found = []
    for backup in game.rglob('*' + SUFFIX):
        _require_safe_file(game, backup)
        target = Path(str(backup)[:-len(SUFFIX)])
        # The original swap only backs up PAKs and the 4096-byte injector shim.
        if target.suffix.casefold() != '.pak' and (
                target.name.casefold() != 'inject.dll' or
                target.parent != game / 'bin64_SteamRetail'):
            raise ValueError('Unexpected backup type: ' + str(backup))
        if (target.name.casefold() == 'inject.dll' and backup.stat().st_size != 4096):
            raise ValueError('Unexpected original injector size: ' + str(backup))
        if target.is_symlink():
            raise ValueError('Cannot overwrite a symbolic link: ' + str(target))
        if target.exists():
            _require_safe_file(game, target)
        if Path(str(target) + '.customkey-ready').exists():
            raise ValueError('Both prepared and original backups exist for ' +
                             str(target) + '; inspect the previous swap before recovery.')
        found.append(RecoveryItem(backup, target, target.is_file(), backup.stat().st_size))
    return sorted(found, key=lambda item: str(item.target).casefold())


def ensure_game_closed() -> None:
    if os.name != 'nt':
        return
    result = subprocess.run(['tasklist', '/FO', 'CSV', '/NH'],
                            capture_output=True, text=True, check=False)
    if result.returncode:
        raise RuntimeError('Could not check running game processes. Close Evolve and retry.')
    names = {line[0].casefold() for line in csv.reader(result.stdout.splitlines()) if line}
    if {'evolve.exe', 'moddedevolvelauncher.exe'} & names:
        raise RuntimeError('Close Evolve.exe and ModdedEvolveLauncher.exe before recovery.')


def _save_manifest(path: Path, record: dict) -> None:
    temporary = path.with_suffix('.json.tmp')
    with temporary.open('x', encoding='utf-8') as stream:
        json.dump(record, stream, indent=2)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)


def recover(game: Path, manifest_dir: Path) -> Path | None:
    """Explicit recovery: preserve live mods, then replace with original backups.

    Never silently overwrite a recovery copy. If a rename fails, attempt an
    immediate per-file rollback and leave the manifest for inspection.
    """
    ensure_game_closed()
    items = plan_recovery(game)
    if not items:
        print('NOTHING TO RESTORE: No original swap backups exist.', flush=True)
        return None
    game = Path(game).resolve(strict=True)
    manifest_dir = Path(manifest_dir).resolve()
    if manifest_dir == game or manifest_dir.is_relative_to(game):
        raise ValueError('Recovery manifest must be saved outside EvolveGame.')
    batch = datetime.now(timezone.utc).strftime('%Y%m%d-%H%M%S') + '-' + uuid.uuid4().hex[:10]
    suffix = '.customkey-recovery-mod-' + batch
    entries = []
    for item in items:
        preserved = Path(str(item.target) + suffix) if item.live_exists else None
        if preserved and (preserved.exists() or preserved.is_symlink()):
            raise ValueError('Recovery name already in use: ' + str(preserved))
        entries.append({'original_backup': str(item.backup),
                        'target': str(item.target),
                        'modified_copy': str(preserved) if preserved else None,
                        'state': 'pending'})
    manifest_dir.mkdir(parents=True, exist_ok=True)
    path = manifest_dir / ('SwapRecovery-' + batch + '.json')
    record = {'version': 1, 'status': 'in_progress', 'game': str(game),
              'note': 'Each original backup becomes the live file. All prior live mod files retain a unique name.',
              'files': entries}
    _save_manifest(path, record)
    print('RECOVERY MANIFEST: ' + str(path), flush=True)
    print('RECOVERY ITEMS: ' + str(len(items)), flush=True)
    for index, (item, entry) in enumerate(zip(items, entries), 1):
        try:
            # Recheck every source just before mutation, including newly created links.
            _require_safe_file(game, item.backup)
            if item.target.is_symlink():
                raise ValueError('Target changed to a symlink: ' + str(item.target))
            if item.live_exists:
                _require_safe_file(game, item.target)
                kept = Path(entry['modified_copy'])
                if kept.exists() or kept.is_symlink():
                    raise ValueError('Existing recovery copy: ' + str(kept))
                os.rename(item.target, kept)
                entry['state'] = 'mod_preserved'
                _save_manifest(path, record)
            else:
                if item.target.exists() or item.target.is_symlink():
                    raise ValueError('Unexpected newly created target: ' + str(item.target))
            os.rename(item.backup, item.target)
            entry['state'] = 'restored'
            _save_manifest(path, record)
        except Exception as error:
            entry['state'] = 'failed'
            entry['error'] = str(error)
            if item.live_exists:
                kept = Path(entry['modified_copy'])
                if kept.is_file() and not item.target.exists() and item.backup.is_file():
                    try:
                        os.rename(kept, item.target)
                        entry['state'] = 'rolled_back'
                    except OSError as rollback_error:
                        entry['rollback_error'] = str(rollback_error)
            record['status'] = 'stopped_for_manual_review'
            _save_manifest(path, record)
            raise RuntimeError('Recovery stopped at ' + str(item.target) +
                               '; see ' + str(path) + ': ' + str(error)) from error
        if index % 25 == 0 or index == len(items):
            print('Restored ' + str(index) + '/' + str(len(items)) + ' files.', flush=True)
    record['status'] = 'complete'
    _save_manifest(path, record)
    print('RECOVERY COMPLETE: ' + str(len(items)) + ' original files restored.', flush=True)
    print('Preserved modified files are alongside the game files with suffix ' + suffix, flush=True)
    return path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game', type=Path, required=True)
    parser.add_argument('--manifest-dir', type=Path)
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    try:
        items = plan_recovery(args.game)
        print('Original backups: ' + str(len(items)), flush=True)
        print('Backup size: %.2f GiB' % (sum(x.bytes for x in items) / (1024 ** 3)), flush=True)
        print('Existing modded files: ' + str(sum(x.live_exists for x in items)), flush=True)
        if not args.apply:
            print('PREVIEW ONLY. No game files changed.', flush=True)
            return 0
        if not args.manifest_dir:
            raise ValueError('--manifest-dir is required for -apply')
        recover(args.game, args.manifest_dir)
        return 0
    except (OSError, RuntimeError, ValueError) as error:
        print('RECOVERY STOPPED: ' + str(error), file=sys.stderr, flush=True)
        return 1


if __name__ == '__main__':
    sys.exit(main())
