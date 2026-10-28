#!/usr/bin/env python3
"""EXPERIMENTAL: reversible custom-key PAK swap at a MANUALLY VERIFIED prelaunch pause.

This tool DOES NOT pause/patch/inject the launcher. DO NOT run `swap` until you
verify a debugger has stopped the launcher immediately before creating Evolve.exe.
The 'prepare' step copies staged archives alongside live files using .ckready
(non-PAK extension) so the paused swap only performs same-volume renames.
"""
import argparse
import csv
import hashlib
import json
import ntpath
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

GAME_DEFAULT = r'C:\Games\ModdedEvolve\EvolveGame'
STAGE_DEFAULT = r'C:\EvolvePakTools\CustomSigning\staged'
READY = '.customkey-ready'
BACKUP = '.customkey-original'
BLOCK = 8 * 1024 * 1024
STATE_FILE = Path(__file__).with_name('controlled_swap_state.json')


def stop(message):
    raise RuntimeError(message)


def sha(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as f:
        for buf in iter(lambda: f.read(BLOCK), b''):
            digest.update(buf)
    return digest.hexdigest()


def read_json(p):
    return json.loads(Path(p).read_text(encoding='utf-8'))


def write_state(state):
    tmp = STATE_FILE.with_suffix('.json.tmp')
    with open(tmp, 'w', encoding='utf-8') as f:
        json.dump(state, f, indent=2)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, STATE_FILE)


def ordinary_windows_path(value):
    """Use a standard Win32 path in journals instead of a \\?\ prefix."""
    text = str(value)
    if text[:8].casefold() == '\\\\?\\unc\\':
        return '\\\\' + text[8:]
    if text.startswith('\\\\?\\'):
        return text[4:]
    return text


def same_game_root(first, second):
    """Old immutable rekey plans may still contain extended drive paths."""
    return (ntpath.normcase(ntpath.normpath(ordinary_windows_path(first))) ==
            ntpath.normcase(ntpath.normpath(ordinary_windows_path(second))))


def task_names():
    if os.name != 'nt':
        return set()
    # The detached auto-restore watchdog calls this every 2 seconds while
    # the game is running. From a GUI/console-free PyInstaller worker, Windows
    # otherwise flashes a new tasklist console for each poll. Do not hide
    # the watchdog itself: it must still monitor and restore original files.
    result = subprocess.run(
        ['tasklist', '/FO', 'CSV', '/NH'],
        capture_output=True, text=True,
        creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
    if result.returncode:
        stop('Could not query processes (tasklist).')
    names = set()
    for row in csv.reader(result.stdout.splitlines()):
        if row: names.add(row[0].casefold())
    return names


def no_game():
    if 'evolve.exe' in task_names():
        stop('Evolve.exe is already running. Swap must occur BEFORE the game process exists.')


def all_closed():
    names = task_names()
    if 'evolve.exe' in names or 'moddedevolvelauncher.exe' in names:
        stop('Close Evolve.exe AND ModdedEvolveLauncher.exe before this operation.')


def selected_signed_entries(stage, plan):
    """Choose only profile-target PAKs; keep the full immutable rekey inventory.

    Missing manifest preserves the legacy full-swap workflow for manual stages.
    A present manifest must be valid; never silently fall back to all PAKs.
    """
    signed = [e for e in plan['entries'] if e.get('kind') == 'signed-encrypted']
    manifest = stage / 'active_mod_paks.json'
    if not manifest.exists() and not manifest.is_symlink():
        return signed
    import stat
    metadata = manifest.lstat()
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > 64 * 1024:
        stop('Unsafe enabled-profile PAK manifest')
    spec = read_json(manifest)
    targets = spec.get('targets')
    if spec.get('schema') != 1 or not isinstance(targets, list) or not 0 < len(targets) <= 512:
        stop('Invalid enabled-profile PAK selection')
    names = set()
    for raw in targets:
        if not isinstance(raw, str) or not raw or len(raw) > 1024:
            stop('Invalid selected PAK path')
        normalized = raw.replace('\\', '/')
        parts = normalized.split('/')
        if (normalized != raw or normalized.startswith('/') or
                any(part in ('', '.', '..') or ':' in part for part in parts) or
                not normalized.lower().endswith('.pak')):
            stop('Unsafe selected PAK path')
        lowered = normalized.casefold()
        if lowered in names:
            stop('Duplicate enabled-profile PAK target')
        names.add(lowered)
    selected = [e for e in signed if str(e.get('path', '')).replace('\\', '/').casefold() in names]
    if len(selected) != len(names):
        stop('Enabled profile references PAKs missing from signed stage')
    return selected


def list_files(game, stage):
    plan = read_json(stage / 'rekey_plan.json')
    status = read_json(stage / 'stage_status.json')
    if status.get('status') != 'fully-staged-offline':
        stop('Stage status is not fully-staged-offline.')
    if plan.get('blocked'):
        stop('Stage plan contains blocked items.')
    if not same_game_root(game, plan.get('source_root', '')):
        stop('Staged game root differs from selected live game root.')
    signed = [e for e in plan['entries'] if e['kind'] == 'signed-encrypted']
    if not signed or len(signed) != status.get('count'):
        stop('Archive count differs from verified stage status.')
    files = []
    for entry in selected_signed_entries(stage, plan):
        rel = Path(entry['path'])
        if rel.is_absolute() or '..' in rel.parts or rel.suffix.lower() != '.pak':
            stop(f'Unsafe archive path: {rel}')
        target = game / rel
        source = stage / 'paks' / rel
        files.append({'name':str(rel), 'target':str(target), 'source':str(source),
                      'ready':str(target) + READY, 'backup':str(target) + BACKUP})
    shim_target = game / 'bin64_SteamRetail' / 'inject.dll'
    shim_source = stage / 'inject-custom.dll'
    files.append({'name':'bin64_SteamRetail/inject.dll', 'target':str(shim_target),
                  'source':str(shim_source), 'ready':str(shim_target) + READY,
                  'backup':str(shim_target) + BACKUP})
    if status.get('signer') == 'official-2025':
        original_debug = game / 'bin64_SteamRetail' / 'dbghelp.dll'
        custom_debug = stage / 'dbghelp-custom.dll'
        if (not original_debug.is_file() or not custom_debug.is_file()
                or sha(original_debug) != status.get('original_dbghelp_sha256', '').lower()
                or sha(custom_debug) != status.get('dbghelp_sha256', '').lower()):
            stop('Official dbghelp.dll original/stage mismatch; restore normal client first.')
        files.append({'name':'bin64_SteamRetail/dbghelp.dll', 'target':str(original_debug),
                      'source':str(custom_debug), 'ready':str(original_debug) + READY,
                      'backup':str(original_debug) + BACKUP})
    return files, status


def fresh(args):
    game = Path(ordinary_windows_path(Path(args.game_root).resolve()))
    stage = Path(ordinary_windows_path(Path(args.stage_dir).resolve()))
    if not game.is_dir() or not stage.is_dir(): stop('Game root or staged folder missing.')
    if os.path.normcase(str(game)) == os.path.normcase(str(stage)) or game in stage.parents or stage in game.parents:
        stop('Stage and game folders must be separate.')
    return game, stage



def check(args):
    game, stage = fresh(args)
    files, status = list_files(game, stage)
    total_bytes = 0
    for item in files:
        source=Path(item['source']);target=Path(item['target'])
        if not source.is_file() or not target.is_file():
            stop(f'Missing staged/installed file: {item["name"]}')
        total_bytes += source.stat().st_size
        if Path(item['backup']).exists():
            stop('Existing live-swap backup present. Run status/restore before another test: '+item['name'])
    oldkey = stage.parent/'RSAKeyData.bin'
    shim=(game/'bin64_SteamRetail'/'inject.dll').read_bytes()
    if not oldkey.exists() or len(shim)!=4096 or shim[0x870:0x870+140]!=oldkey.read_bytes():
        stop('Installed inject.dll does not match original public key.')
    print(f'CHECK PASSED: {status["count"]} signed archives and matching runtime DLLs are available.')
    print(f'Estimated temporary staging volume: {total_bytes/1024**3:.2f} GiB plus reserve.')
    print(f'Free space on game volume: {shutil.disk_usage(game).free/1024**3:.2f} GiB.')
    print('No game files changed. Next: verify x64dbg can pause BEFORE Evolve.exe creation.')

def preserve_abandoned_partial(partial, target, ready, backup):
    """Quarantine only an orphaned preparation artifact, never an original PAK.

    Called from prepare only after the game/launcher-close, restored-journal,
    stage-identity, source/target presence and *all* backup checks have passed.
    Do not resume unverified partial bytes or remove a recovery backup.
    """
    if backup.exists() or backup.is_symlink():
        stop(f'Original game backup exists; restore before preparing: {backup}')
    if not target.is_file() or target.is_symlink():
        stop(f'Installed original is missing or unsafe: {target}')
    if ready.exists() or ready.is_symlink():
        stop(f'Cannot quarantine partial beside an existing prepared file: {ready}')

    import stat
    metadata = partial.lstat()
    reparse = getattr(metadata, 'st_file_attributes', 0) & getattr(
        stat, 'FILE_ATTRIBUTE_REPARSE_POINT', 0
    )
    if not stat.S_ISREG(metadata.st_mode) or reparse or metadata.st_size > 16 * 1024**3:
        stop(f'Unsafe abandoned prepared file; manual inspection required: {partial}')

    # Same-directory rename preserves bytes without copying another full PAK.
    # This path has a unique, non-PAK suffix and is never used as a signing input.
    for attempt in range(64):
        preserved = partial.with_name(
            f'{partial.name}.stale-{time.time_ns()}-{attempt}'
        )
        if preserved.exists() or preserved.is_symlink():
            continue
        try:
            partial.rename(preserved)
        except FileExistsError:
            continue
        except OSError as error:
            stop(f'Could not preserve abandoned prepared file {partial}: {error}')
        print(f'Preserved interrupted preparation file: {preserved}')
        return preserved
    stop(f'Could not allocate a distinct preservation name for: {partial}')


def prepared_slot_is_regular(path):
    """Reject symlinks, reparse points and oversized temporary copies."""
    import stat
    try:
        meta = path.lstat()
    except FileNotFoundError:
        return False
    reparse = getattr(meta, 'st_file_attributes', 0) & getattr(
        stat, 'FILE_ATTRIBUTE_REPARSE_POINT', 0
    )
    if not stat.S_ISREG(meta.st_mode) or reparse or meta.st_size > 16 * 1024**3:
        stop(f'Unsafe prepared copy; no game files changed: {path}')
    return True


def preserve_outdated_ready_copy(item):
    """Retire only a verified temporary READY file, never a stock PAK/backup."""
    ready = Path(item['ready'])
    target = Path(item['target'])
    backup = Path(item['backup'])
    if backup.exists() or backup.is_symlink():
        stop(f'Original backup still exists; restore before refreshing: {backup}')
    if not target.is_file() or target.is_symlink():
        stop(f'Installed game original missing or unsafe: {target}')
    if not prepared_slot_is_regular(ready):
        stop(f'Prepared copy disappeared before it could be preserved: {ready}')
    for attempt in range(64):
        preserved = ready.with_name(
            f'{ready.name}.stale-{time.time_ns()}-{attempt}'
        )
        if preserved.exists() or preserved.is_symlink():
            continue
        try:
            ready.rename(preserved)
        except FileExistsError:
            continue
        except OSError as error:
            stop(f'Could not preserve outdated prepared copy {ready}: {error}')
        print(f'Preserved outdated profile prepared copy: {preserved}', flush=True)
        return preserved
    stop(f'Could not allocate a safe stale-copy name for: {ready}')


def prepare(args):
    all_closed()
    if STATE_FILE.exists():
        s = read_json(STATE_FILE)
        if s.get('phase') not in ('restored',):
            stop('An existing state journal exists. Run status/restore first; do not overwrite its backups.')
        # A second prepare is valid only if the previous run has no backup files
        if any(Path(x['backup']).exists() for x in s.get('files', [])):
            stop('Leftover backup files exist; cannot create a new journal.')
    game, stage = fresh(args)
    files, status = list_files(game, stage)
    old_key_file = stage.parent / 'RSAKeyData.bin'
    old_key = old_key_file.read_bytes() if old_key_file.is_file() else b''
    game_shim = (game / 'bin64_SteamRetail' / 'inject.dll').read_bytes()
    if len(old_key) != 140 or len(game_shim) != 4096 or game_shim[0x870:0x870+140] != old_key:
        stop('Installed inject.dll does not contain the original public key. Restore normal shim first.')
    custom_shim_file = stage / 'inject-custom.dll'
    if sha(custom_shim_file) != status.get('shim_sha256','').lower():
        stop('Staged custom shim SHA256 differs from the verified staging report.')
    # A previous profile can leave behind READY files no longer listed in
    # the restored journal. Validate the full selection and ORIGINAL targets
    # before retiring any temporary copy; never overwrite a live/unknown PAK.
    plan = read_json(stage / 'rekey_plan.json')
    original_hashes = {
        str(entry['path']).replace('\\', '/').casefold(): entry.get('sha256')
        for entry in plan['entries'] if entry.get('kind') == 'signed-encrypted'
    }
    required = 0
    reusable = {}
    outdated = set()
    for item in files:
        source = Path(item['source']); target = Path(item['target'])
        ready = Path(item['ready']); backup = Path(item['backup'])
        if (not target.is_file() or target.is_symlink()
                or not source.is_file() or source.is_symlink()):
            stop(f'Missing or unsafe installed/staged file: {item["name"]}')
        if backup.exists() or backup.is_symlink():
            stop(f'Original backup remains; restore before preparing: {backup}')
        if prepared_slot_is_regular(ready):
            source_hash = sha(source)
            if ready.stat().st_size == source.stat().st_size and sha(ready) == source_hash:
                reusable[item['name']] = source_hash
                continue
            # Only stale, non-PAK ready copies are recyclable. Confirm the
            # installed stock archive still hashes to its original setup
            # digest; the injector and optional dbghelp were checked above.
            if item['name'].casefold().endswith('.pak'):
                original_hash = original_hashes.get(
                    item['name'].replace('\\', '/').casefold()
                )
                if (not isinstance(original_hash, str)
                        or len(original_hash) != 64
                        or any(c not in '0123456789abcdef' for c in original_hash.lower())
                        or sha(target).lower() != original_hash.lower()):
                    stop(f'Installed stock PAK differs from signing inventory: {item["name"]}')
            outdated.add(item['name'])
        required += source.stat().st_size
    if os.path.splitdrive(str(game))[0].casefold() != os.path.splitdrive(str(stage))[0].casefold() and os.name=='nt':
        print('NOTE: staged PAKs on another volume; prepare copies them onto the game volume.')
    free = shutil.disk_usage(game).free
    if free < required + 1024**3:
        stop(f'Insufficient free space for prepared copies: need {(required + 1024**3)/1024**3:.2f} GiB, have {free/1024**3:.2f} GiB.')
    print(f'Preparing {len(files)} files; {required/1024**3:.2f} GiB copied to non-PAK temporary names.')
    # Full hash each source during copy, then verify the destination bytes independently.
    for i, item in enumerate(files, 1):
        source = Path(item['source']); ready = Path(item['ready'])
        if item['name'] in reusable:
            # Revalidate at publication time; a previous preflight hash alone
            # cannot authorize a ready file if its bytes changed afterward.
            verified_digest = sha(source)
            if (verified_digest != reusable[item['name']]
                    or not prepared_slot_is_regular(ready)
                    or sha(ready) != verified_digest):
                stop(f'Prepared copy changed during verification: {ready}')
        else:
            if item['name'] in outdated:
                preserve_outdated_ready_copy(item)
            ready.parent.mkdir(parents=True, exist_ok=True)
            partial = Path(str(ready) + '.partial')
            if partial.exists() or partial.is_symlink():
                preserve_abandoned_partial(
                    partial, Path(item['target']), ready, Path(item['backup'])
                )
            source_digest = hashlib.sha256()
            with source.open('rb') as inp, partial.open('xb') as out:
                for b in iter(lambda: inp.read(BLOCK), b''):
                    source_digest.update(b); out.write(b)
                out.flush(); os.fsync(out.fileno())
            # Verify bytes from disk, not just the stream used for copying.
            verified_digest = source_digest.hexdigest()
            if verified_digest != sha(partial):
                stop(f'Prepared file hash mismatch: {item["name"]}')
            os.replace(partial, ready)
        item['sha256'] = verified_digest
        item['bytes'] = ready.stat().st_size
        if i % 25 == 0 or i == len(files):print(f'Prepared {i}/{len(files)}')
    # Specific key check: custom shim must contain the staged public key at 0x870
    newshim = (stage/'inject-custom.dll').read_bytes()
    newkey = (stage/'mykeys'/'public_key.bin').read_bytes()
    if len(newshim) != 4096 or len(newkey) != 140 or newshim[0x870:0x870+140] != newkey:
        stop('Custom inject.dll RSA key bytes do not match staged public key.')
    state = {'version':1, 'phase':'prepared', 'game':str(game), 'stage':str(stage),
             'files':files, 'count':len(files), 'created_at':time.strftime('%Y-%m-%d %H:%M:%S')}
    write_state(state)
    print('PREPARED. Game originals untouched. Keep game and launcher closed until setting the debugger breakpoint.')
    print('DO NOT RUN swap until you have paused the launcher at the Evolve.exe CreateProcessW call.')


def require_state():
    if not STATE_FILE.exists():stop('No swap journal. Run prepare first.')
    state = read_json(STATE_FILE)
    if state.get('version') != 1:stop('Unrecognized state format.')
    return state


def swap(args):
    state = require_state()
    if state['phase'] != 'prepared':stop(f'Cannot swap in phase {state["phase"]}; restore first.')
    if not args.confirm_launcher_paused:
        stop('Missing --confirm-launcher-paused. Only use when debugger BREAKS at game process creation, BEFORE Evolve.exe exists.')
    no_game()
    for item in state['files']:
        if not Path(item['target']).is_file() or not Path(item['ready']).is_file() or Path(item['backup']).exists():
            stop('Pre-swap file missing or backup already exists: ' + item['name'])
        if Path(item['ready']).stat().st_size != item['bytes']:
            stop('Prepared file size changed since prepare: '+item['name'])
    state['phase']='swapping';write_state(state)
    print('Swapping archived PAKs and shim now; DO NOT resume debugger until swap completes.')
    try:
        for i,item in enumerate(state['files'],1):
            os.replace(item['target'],item['backup'])
            os.replace(item['ready'],item['target'])
            if i%25==0 or i==len(state['files']):print(f'Swapped {i}/{len(state["files"])}')
        state['phase']='swapped';write_state(state)
        print('SWAP COMPLETE. Resume the paused launcher now; after the game test, CLOSE both apps then restore.')
    except Exception as e:
        state['phase']='interrupted';write_state(state)
        print('SWAP INTERRUPTED. Leave launcher paused and run restore once both apps are closed.', file=sys.stderr)
        raise


def verify(args):
    state=require_state()
    if state['phase'] not in ('swapped','interrupted'):
        stop('No active swap to verify.')
    for i,item in enumerate(state['files'],1):
        target=Path(item['target'])
        if not target.is_file() or sha(target)!=item['sha256']:
            stop('Installed file differs from prepared staged copy: '+item['name'])
        if i%25==0 or i==len(state['files']):print(f'Verified {i}/{len(state["files"])}')
    print('PASS: live installed files match prepared custom-signed files; this does not prove game/launcher compatibility.')


def restore(args):
    all_closed()
    state=require_state()
    print('Restoring originals from on-disk backup files...')
    count=0
    for item in reversed(state['files']):
        original=Path(item['backup']);target=Path(item['target'])
        if original.is_file():
            # only the prepare/swap tool creates this suffix; restore from it even after a repair
            os.replace(original,target)
            count+=1
    state['phase']='restored'
    write_state(state)
    print(f'RESTORED {count} original file(s). Remaining prepared copies were left in place for inspection.')
    print('Once this succeeds, you may launch normally and use the normal Repair function if needed.')


def status(args):
    state=require_state()
    backups=sum(Path(item['backup']).exists() for item in state['files'])
    pending=sum(Path(item['ready']).exists() for item in state['files'])
    print('State:',state['phase'])
    print('File count:',len(state['files']))
    print('Original backups:',backups)
    print('Prepared copies remaining:',pending)
    print('Game root:',state['game'])
    print('Stage root:',state['stage'])


def main():
    p=argparse.ArgumentParser(description=__doc__,formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('command',choices=('check','prepare','swap','verify','restore','status'))
    p.add_argument('--game-root',default=GAME_DEFAULT)
    p.add_argument('--stage-dir',default=STAGE_DEFAULT)
    p.add_argument('--confirm-launcher-paused',action='store_true')
    args=p.parse_args()
    try:
        {'check':check,'prepare':prepare,'swap':swap,'verify':verify,'restore':restore,'status':status}[args.command](args)
        return 0
    except Exception as e:
        print('STOPPED:',type(e).__name__,str(e),file=sys.stderr)
        return 1

if __name__=='__main__':sys.exit(main())
