#!/usr/bin/env python3
"""Recover originals and KEEP modded PAKs prepared for the next automatic run.
Only run after closing both Evolve.exe and ModdedEvolveLauncher.exe.
"""
import sys
import os
from pathlib import Path
import controlled_swap as swap

def main():
    try:
        swap.all_closed()
        state=swap.require_state()
        if state['phase']=='prepared':
            print('Already prepared. No restoration needed.')
            return 0
        if state['phase']=='restored':
            print('Originals already restored; use 01-Prepare.ps1 for a fresh staging copy.')
            return 0
        if state['phase'] not in ('swapped','interrupted','restoring'):
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
