#!/usr/bin/env python3
"""Recover originals and KEEP modded PAKs prepared for the next automatic run.
Only run after closing both Evolve.exe and ModdedEvolveLauncher.exe.
"""
import sys
import os
from pathlib import Path
import controlled_swap as swap


def _validate_stranded_clean_state(state):
    """Only recover a journal that says clean when every stranded original
    backup still matches the signed source baseline. Never guess filenames
    or dispose of an unverified game file.
    """
    stage = Path(state['stage']).resolve(strict=True)
    game = Path(state['game']).resolve(strict=True)
    plan = swap.read_json(stage / 'rekey_plan.json')
    if Path(plan['source_root']).resolve(strict=True) != game:
        raise RuntimeError('Recovery journal and signed game baseline differ')
    signed = [entry for entry in plan['entries']
              if entry.get('kind') == 'signed-encrypted']
    expected = [str(entry['path']).replace('\\', '/').casefold() for entry in signed]
    expected.append('bin64_steamretail/inject.dll')
    if len(expected) != len(state['files']) or len(set(expected)) != len(expected):
        raise RuntimeError('Recovery journal does not match signed PAK inventory')
    old_key = (stage.parent / 'RSAKeyData.bin').read_bytes()
    if len(old_key) != 140:
        raise RuntimeError('Original injector verification key missing')
    for item, name in zip(state['files'], expected):
        actual = str(item['name']).replace('\\', '/').casefold()
        if actual != name:
            raise RuntimeError('Stranded backup entry differs from signed inventory')
        relative = Path(*name.split('/'))
        if relative.is_absolute() or '..' in relative.parts or ':' in name:
            raise RuntimeError('Unsafe original member path')
        target = Path(item['target'])
        backup = Path(item['backup'])
        ready = Path(item['ready'])
        expected_target = game / relative
        expected_source = (stage / 'inject-custom.dll'
                           if name == 'bin64_steamretail/inject.dll'
                           else stage / 'paks' / relative)
        source = Path(item['source'])
        if (target.resolve(strict=False) != expected_target.resolve(strict=False)
                or source.resolve(strict=True) != expected_source.resolve(strict=True)
                or backup != Path(str(target) + swap.BACKUP)
                or ready != Path(str(target) + swap.READY)
                or ready.is_symlink()):
            raise RuntimeError('Unsafe stranded original recovery path')
        if not backup.exists():
            continue
        if backup.is_symlink() or not backup.is_file():
            raise RuntimeError('Original backup is not a regular file: ' + str(backup))
        if name == 'bin64_steamretail/inject.dll':
            original = backup.read_bytes()
            if len(original) != 4096 or original[0x870:0x870 + 140] != old_key:
                raise RuntimeError('Stranded injector backup does not match original RSA key')
        else:
            entry = signed[expected.index(name)]
            recorded = entry.get('sha256', '')
            if (len(recorded) != 64
                    or any(c not in '0123456789abcdef' for c in recorded.lower())
                    or swap.sha(backup).lower() != recorded.lower()):
                raise RuntimeError('Stranded PAK backup differs from recorded game original: ' + name)
        # If the installed target already holds original bytes, keep both
        # untouched for manual inspection instead of quarantining good data.
        if target.is_file() and swap.sha(target) == swap.sha(backup):
            raise RuntimeError('Installed original and backup coexist; inspect before recovery: ' + name)


def main():
    try:
        swap.all_closed()
        state=swap.require_state()
        backups = any(Path(item['backup']).exists() for item in state['files'])
        if state['phase'] in ('prepared', 'restored'):
            if not backups:
                print('Originals are already restored; no backup recovery needed.')
                return 0
            # Interrupted/superseded journal state after a swap. This is an
            # explicit recovery action, not permission to start another game.
            _validate_stranded_clean_state(state)
            print('Verified stranded originals in a clean-phase journal; restoring...', flush=True)
        elif state['phase'] not in ('swapping', 'swapped', 'interrupted', 'restoring'):
            raise RuntimeError('Unexpected phase '+str(state['phase']))
        # Preflight for dangerous unexpected files before mutating anything.
        for item in state['files']:
            target=Path(item['target']);backup=Path(item['backup']);ready=Path(item['ready'])
            if backup.exists() and ready.exists() and target.exists():
                raise RuntimeError(f'Prepared and live copy coexist with original backup; inspect before restore: {item["name"]}')
        state['phase']='restoring'
        swap.write_state(state)
        restored=0;quarantined=0;preserved=0
        for i,item in enumerate(reversed(state['files']),1):
            target=Path(item['target']);backup=Path(item['backup']);ready=Path(item['ready'])
            if backup.exists():
                if target.is_file():
                    # Avoid preserving launcher-repaired or truncated modded content.
                    good=(target.stat().st_size == item['bytes'] and swap.sha(target)==item['sha256'])
                    if good:
                        if ready.exists():
                            raise RuntimeError('Prepared slot unexpectedly exists: '+str(ready))
                        os.replace(target, ready)
                        preserved+=1
                    else:
                        unexpected=Path(str(target)+'.customkey-unexpected')
                        if unexpected.exists():
                            raise RuntimeError('Quarantine file exists, abort: '+str(unexpected))
                        os.replace(target,unexpected)
                        quarantined+=1
                        print('Quarantined altered file: '+str(unexpected))
                os.replace(backup,target)
                restored+=1
            if i%25==0 or i==len(state['files']):
                print(f'Processed {i}/{len(state["files"])}; originals restored {restored}',flush=True)
        ready_count=sum(Path(it['ready']).is_file() for it in state['files'])
        backup_count=sum(Path(it['backup']).exists() for it in state['files'])
        if backup_count:
            raise RuntimeError(str(backup_count)+' original backups remain; keep journal and retry.')
        if ready_count==len(state['files']) and quarantined==0:
            state['phase']='prepared'
            print('RESTORED ORIGINALS and RECYCLED ALL '+str(ready_count)+' custom files. Ready for next one-click launch.')
        else:
            state['phase']='restored'
            print('RESTORED ORIGINALS, but '+str(len(state['files'])-ready_count)+' custom file(s) need fresh preparation.')
            print('Run 01-Prepare.ps1 before the next automatic launch.')
        swap.write_state(state)
        return 0
    except Exception as e:
        print('RESTORE STOPPED:',str(e),file=sys.stderr)
        print('Keep backups and journal. Do not run the launcher until originals are restored.',file=sys.stderr)
        return 1
if __name__=='__main__':sys.exit(main())
