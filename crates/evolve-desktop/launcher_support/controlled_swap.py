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


def list_files(game, stage):
    plan = read_json(stage / 'rekey_plan.json')
    status = read_json(stage / 'stage_status.json')
    if status.get('status') != 'fully-staged-offline':
        stop('Stage status is not fully-staged-offline.')
    if plan.get('blocked'):
        stop('Stage plan contains blocked items.')
    if os.path.normcase(os.path.normpath(str(game))) != os.path.normcase(os.path.normpath(plan.get('source_root',''))):
        stop('Staged game root differs from selected live game root.')
    signed = [e for e in plan['entries'] if e['kind'] == 'signed-encrypted']
    if not signed or len(signed) != status.get('count'):
        stop('Archive count differs from verified stage status.')
    files = []
    for entry in signed:
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
    return files, status


def fresh(args):
    game = Path(args.game_root).resolve()
    stage = Path(args.stage_dir).resolve()
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
    print(f'CHECK PASSED: {len(files)-1} signed archives and one custom RSA shim are available.')
    print(f'Estimated temporary staging volume: {total_bytes/1024**3:.2f} GiB plus reserve.')
    print(f'Free space on game volume: {shutil.disk_usage(game).free/1024**3:.2f} GiB.')
    print('No game files changed. Next: verify x64dbg can pause BEFORE Evolve.exe creation.')

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
    required = 0
    for item in files:
        source = Path(item['source']); target = Path(item['target'])
        if not target.is_file() or not source.is_file(): stop(f'Missing installed or staged file: {item["name"]}')
        if Path(item['backup']).exists(): stop(f'Existing backup: {item["backup"]}')
        if not Path(item['ready']).exists(): required += source.stat().st_size
    if os.path.splitdrive(str(game))[0].casefold() != os.path.splitdrive(str(stage))[0].casefold() and os.name=='nt':
        print('NOTE: staged PAKs on another volume; prepare copies them onto the game volume.')
    free = shutil.disk_usage(game).free
    if free < required + 1024**3:
        stop(f'Insufficient free space for prepared copies: need {(required + 1024**3)/1024**3:.2f} GiB, have {free/1024**3:.2f} GiB.')
    print(f'Preparing {len(files)} files; {required/1024**3:.2f} GiB copied to non-PAK temporary names.')
    # Full hash each source during copy, then verify the destination bytes independently.
    for i, item in enumerate(files, 1):
        source = Path(item['source']); ready = Path(item['ready'])
        if ready.exists():
            if source.stat().st_size != ready.stat().st_size:
                stop(f'Existing prepared file differs from staged source: {ready}')
            # Both copies are fully rehashed before any existing cache is
            # trusted. Keep the verified digest; do not read READY a second time.
            verified_digest = sha(source)
            if verified_digest != sha(ready):
                stop(f'Existing prepared file differs from staged source: {ready}')
        else:
            ready.parent.mkdir(parents=True, exist_ok=True)
            partial = Path(str(ready) + '.partial')
            if partial.exists():
                stop(f'Leftover partial file requires manual inspection: {partial}')
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
