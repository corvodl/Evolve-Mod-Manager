#!/usr/bin/env python3
"""Experimental Evolve Stage 2 swap helper using Frida instrumentation.

This intentionally leaves the existing auto_launch.py and controlled_swap.py
untouched. Frida runs a CreateProcessW onEnter hook inside the user-owned launcher
and waits while the existing reversible prepared-file swap is carried out.
Use for private/offline testing only. No modification before verified game launch.
"""
from __future__ import annotations

import ctypes
import os
from pathlib import Path
import subprocess
import sys
import threading
import time

import controlled_swap as swapper
import standard_user_launch

LAUNCHER = Path(r'C:\Users\billn\AppData\Local\Programs\Modded Evolve\ModdedEvolveLauncher.exe')
GAME_EXE = Path(r'C:\Games\ModdedEvolve\EvolveGame\bin64_SteamRetail\Evolve.exe')
WAIT_SECONDS = 900


def canonical_exec_path(path: str) -> str:
    """Compare drive/UNC paths independently of the Win32 extended prefix.

    Do not collapse the leading two UNC separators; doing so can turn a
    network root into a different path. Only path spelling is normalized,
    never the executable basename or its command-line argument boundary.
    """
    path = (path or '').casefold().replace('/', '\\')
    extended_unc = '\\\\?\\unc\\'
    extended = '\\\\?\\'
    if path.startswith(extended_unc):
        path = '\\\\' + path[len(extended_unc):]
    elif path.startswith(extended):
        path = path[len(extended):]
    unc = path.startswith('\\\\')
    prefix = '\\\\' if unc else ''
    body = path[2:] if unc else path
    while '\\\\' in body:
        body = body.replace('\\\\', '\\')
    return prefix + body


def normalize_exec_command(command: str) -> bool:
    if not isinstance(command, str) or not command.strip():
        return False
    clean = command.strip()
    expected = canonical_exec_path(str(GAME_EXE))
    if clean.startswith('"'):
        end = clean.find('"', 1)
        if end <= 1 or (end + 1 < len(clean) and not clean[end + 1].isspace()):
            return False
        return canonical_exec_path(clean[1:end]) == expected
    candidate = canonical_exec_path(clean)
    return candidate == expected or (
        candidate.startswith(expected)
        and len(candidate) > len(expected)
        and candidate[len(expected)].isspace()
    )

def is_evolve_launch(app: str, cmd: str) -> bool:
    return normalize_exec_command(app if app else cmd)


def preflight() -> None:
    if os.name != 'nt' or sys.maxsize <= 2**32:
        raise RuntimeError('Requires 64-bit Windows and 64-bit Python 3.11.')
    global LAUNCHER, GAME_EXE
    import json
    config_path = Path(__file__).with_name('launcher_config.json')
    if config_path.is_file():
        config = json.loads(config_path.read_text(encoding='utf-8'))
        if config.get('launcher'): LAUNCHER = Path(config['launcher'])
    state = swapper.require_state()
    GAME_EXE = Path(swapper.ordinary_windows_path(state['game'])) / 'bin64_SteamRetail' / 'Evolve.exe'
    if (Path(state['stage'])/'REFRESHED-TO.json').exists():
        raise RuntimeError('Stage was superseded. Use the refreshed signing stage.')
    if not LAUNCHER.is_file() or not GAME_EXE.is_file():
        raise RuntimeError('Original launcher or Evolve.exe not found. Check README path settings.')
    swapper.all_closed()
    state = swapper.require_state()
    if state['phase'] != 'prepared':
        raise RuntimeError(f'Swap state is {state["phase"]}; restore and prepare first.')
    expected, _ = swapper.list_files(Path(state['game']), Path(state['stage']))
    if [f['name'] for f in state['files']] != [f['name'] for f in expected]:
        raise RuntimeError('Prepared file list differs from current stage plan.')
    pending = [f['name'] for f in state['files'] if
               not Path(f['ready']).is_file() or not Path(f['target']).is_file()
               or Path(f['backup']).exists()
               or Path(f['ready']).stat().st_size != f['bytes']]
    if pending:
        raise RuntimeError('Prepared state inconsistent for ' + str(pending[:3]))
    print(f'PRECHECK PASS: {len(state["files"])-1} signed PAKs + matching shim prepared; originals untouched.', flush=True)


FRIDA_JS = r'''
'use strict';
const path = __GAME_PATH__;
let blocked = false;
let seq = 0;
function canon(raw) {
  let p = (raw || '').toLowerCase().replaceAll('/', '\\');
  const extendedUnc = '\\\\?\\unc\\';
  const extended = '\\\\?\\';
  if (p.startsWith(extendedUnc)) p = '\\\\' + p.slice(extendedUnc.length);
  else if (p.startsWith(extended)) p = p.slice(extended.length);
  const unc = p.startsWith('\\\\');
  const prefix = unc ? '\\\\' : '';
  let body = unc ? p.slice(2) : p;
  while (body.includes('\\\\')) body = body.replaceAll('\\\\', '\\');
  return prefix + body;
}
function matches(value) {
  if (!value) return false;
  const candidate = value.trim();
  const expected = canon(path);
  if (candidate.startsWith('"')) {
    const end = candidate.indexOf('"', 1);
    if (end <= 1 || (end + 1 < candidate.length && !/\s/.test(candidate[end + 1])))
      return false;
    return canon(candidate.slice(1, end)) === expected;
  }
  const normalized = canon(candidate);
  return normalized === expected ||
    (normalized.startsWith(expected) && /\s/.test(normalized[expected.length] || ''));
}
function readWide(p) {
  if (p.isNull()) return '';
  try { return p.readUtf16String(8192) || ''; }
  catch (e) { return ''; }
}
try {
  const kb = Process.getModuleByName('KERNELBASE.dll');
  const address = kb.getExportByName('CreateProcessW');
  Interceptor.attach(address, {
    onEnter(args) {
      if (blocked) return;
      const app = readWide(args[0]);
      const cmd = readWide(args[1]);
      if (!matches(app || cmd)) return;
      blocked = true;
      const token = ++seq;
      send({kind:'candidate', token, app, cmd});
      let allowed = false;
      recv('resume', function (reply) {
        const v = reply.payload || {};
        allowed = v.token === token && v.allowed === true;
      }).wait();
      if (!allowed) {
        send({kind:'blocked', token});
        // Do not let an unverified/failed swap proceed into the game startup.
        // Host should terminate this launcher; if still alive, remain blocked.
        recv('resume', function (_) {}).wait();
      }
      send({kind:'released', token});
    }
  });
  send({kind:'ready', address:address.toString()});
} catch (e) {
  send({kind:'hook_error', error:String(e), stack: e.stack || ''});
}
'''


def make_js() -> str:
    import json
    return FRIDA_JS.replace('__GAME_PATH__', json.dumps(str(GAME_EXE)))


class SwapController:
    def __init__(self, proc, script):
        self.proc = proc
        self.script = script
        self.ready = threading.Event()
        self.finished = threading.Event()
        self.failed = None
        self.swapped = False
        self.launch_count = 0

    def on_message(self, message, data):
        if message.get('type') == 'error':
            self.failed = 'Frida agent error: ' + message.get('stack', str(message))
            self.finished.set()
            return
        if message.get('type') != 'send':
            return
        payload = message.get('payload') or {}
        kind = payload.get('kind')
        if kind == 'ready':
            print('Automatic CreateProcessW hook ready at ' + str(payload.get('address')), flush=True)
            self.ready.set()
        elif kind == 'hook_error':
            self.failed = 'Could not install CreateProcessW hook: ' + str(payload.get('error'))
            self.finished.set()
        elif kind == 'candidate':
            self.launch_count += 1
            app = payload.get('app') or ''
            cmd = payload.get('cmd') or ''
            token = payload.get('token')
            print('CreateProcessW candidate: ' + (app or cmd)[:350], flush=True)
            if (not isinstance(token, int) or not is_evolve_launch(app, cmd)
                    or self.launch_count != 1):
                self.failed = 'Unexpected or unverified Evolve process-creation call; no swap.'
                self._terminate()
                self.finished.set()
                return
            try:
                print('Evolve creation paused inside launcher; swapping prepared archives...', flush=True)
                from types import SimpleNamespace
                swapper.swap(SimpleNamespace(confirm_launcher_paused=True))
                self.swapped = True
                print('SWAP COMPLETE. Releasing CreateProcessW...', flush=True)
                self.script.post({'type':'resume','payload':{'token':token,'allowed':True}})
            except Exception as exc:
                self.failed = f'Swap failed: {type(exc).__name__}: {exc}. Launcher will be stopped.'
                self._terminate()
                self.finished.set()
        elif kind == 'released':
            if self.swapped:
                print('Evolve creation released successfully.', flush=True)
                self.finished.set()
        elif kind == 'blocked':
            self.failed = 'Agent refused to release Evolve creation.'
            self._terminate()
            self.finished.set()

    def _terminate(self):
        try:
            if self.proc.poll() is None:
                self.proc.terminate()
        except Exception as exc:
            print(f'WARNING: could not stop launcher: {exc}', file=sys.stderr)


def start_restore_watchdog():
    """Start an independent watchdog after a verified swap, not before."""
    helper = Path(__file__).with_name("auto_restore_watch.py")
    if not helper.is_file():
        raise FileNotFoundError("Auto-restoration helper was not installed.")
    command = [sys.executable]
    if not getattr(sys, "frozen", False):
        command.append("-u")
    command.append(str(helper))
    flags = (getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0) |
             getattr(subprocess, "DETACHED_PROCESS", 0))
    with Path(__file__).with_name("auto-restore.log").open("a", encoding="utf-8") as out:
        subprocess.Popen(command, cwd=str(helper.parent),
                         stdin=subprocess.DEVNULL, stdout=out,
                         stderr=subprocess.STDOUT, close_fds=True,
                         creationflags=flags)


def main() -> int:
    launcher = None
    session = None
    script = None
    ctl = None
    try:
        preflight()
        try:
            import frida
        except ImportError:
            raise RuntimeError('Frida is not installed. Run Install-Frida.cmd first.') from None
        # The elevated helper retains permission to perform atomic PAK swaps,
        # but the interactive client and its Evolve.exe child must run under
        # Explorer's verified MEDIUM-integrity user token. Never silently
        # fall back to an elevated subprocess on failure.
        own_privileges = standard_user_launch.get_elevation()
        launcher = standard_user_launch.launch_standard_user(LAUNCHER)
        actual_privileges = standard_user_launch.get_elevation(launcher.pid)
        print(
            f"Privilege diagnostic: swap_helper_pid={os.getpid()} elevated={own_privileges['elevated']}; "
            f"client_pid={launcher.pid} elevated={actual_privileges['elevated']} "
            f"integrity=0x{actual_privileges['integrity_rid']:x}",
            flush=True,
        )
        print(f'Launcher PID {launcher.pid} (standard user, medium integrity)', flush=True)
        # Let the host launch, then instrument it before the user presses Play.
        session = frida.attach(launcher.pid)
        script = session.create_script(make_js())
        ctl = SwapController(launcher, script)
        script.on('message', ctl.on_message)
        script.load()
        deadline = time.monotonic() + WAIT_SECONDS
        while time.monotonic() < deadline:
            if ctl.finished.wait(0.2): break
            if launcher.poll() is not None:
                raise RuntimeError('Launcher exited before Evolve creation was intercepted.')
            if not ctl.ready.is_set() and time.monotonic() + 0.2 >= deadline:
                raise TimeoutError('Timed out installing the launcher hook.')
        if ctl.failed:
            raise RuntimeError(ctl.failed)
        if not ctl.finished.is_set():
            raise TimeoutError('Timed out waiting for Evolve launch. No swap completed.')
        if not ctl.swapped:
            raise RuntimeError('Evolve launch was not authorized; no swap completed.')
        # The hook has already unblocked and sent its final signal. Give onEnter
        # a moment to return before detaching and unloading the interceptor.
        time.sleep(0.3)
        print('Evolve is starting. After playing CLOSE launcher + game, then restore originals.', flush=True)
        return 0
    except Exception as exc:
        print('AUTOMATIC LAUNCH FAILED:', type(exc).__name__, str(exc), file=sys.stderr)
        print('Check 03-Status.ps1. If swapping/interrupted, close both apps before restore.',file=sys.stderr)
        return 1
    finally:
        # Even if the hook errors after swapping, keep waiting for both
        # processes to exit, then attempt the existing verified restoration.
        if ctl is not None and ctl.swapped:
            try:
                start_restore_watchdog()
                print("Independent auto-restore monitor armed.", flush=True)
            except Exception as error:
                print("AUTO-RESTORE MONITOR FAILED: " + str(error),
                      file=sys.stderr, flush=True)
        if session is not None:
            try: session.detach()
            except Exception: pass
        if launcher is not None:
            try: launcher.close()
            except Exception: pass


if __name__ == '__main__':
    sys.exit(main())
