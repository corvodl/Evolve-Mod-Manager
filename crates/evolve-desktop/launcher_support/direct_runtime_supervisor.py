#!/usr/bin/env python3
"""Detached managed Stage2 service lifecycle for manager-owned Evolve process.

This host owns a *private copy* of the exact installed service implementation.
It never launches Evolve itself and never ships identity/profile credentials.
"""
from __future__ import annotations
import argparse
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import traceback

import client_service_host
import direct_standard_user_launch as broker
import stage2_runtime_contract as contract


def atomic_json(path, obj):
    path=Path(path)
    temp=path.with_suffix(".tmp")
    temp.write_text(json.dumps(obj,indent=2), encoding="utf-8")
    os.replace(temp,path)


def running_game(pid):
    """Check the actual kernel process status, not an unrelated PID in tasklist."""
    if os.name!="nt":
        return False
    kernel=ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.restype=wintypes.HANDLE
    kernel.OpenProcess.argtypes=(wintypes.DWORD,wintypes.BOOL,wintypes.DWORD)
    kernel.GetExitCodeProcess.restype=wintypes.BOOL
    kernel.GetExitCodeProcess.argtypes=(wintypes.HANDLE,ctypes.POINTER(wintypes.DWORD))
    kernel.CloseHandle.argtypes=(wintypes.HANDLE,)
    handle=kernel.OpenProcess(0x1000,False,int(pid))
    if not handle:
        return False
    try:
        status=wintypes.DWORD()
        return bool(kernel.GetExitCodeProcess(handle,ctypes.byref(status))) and status.value==259
    finally:
        kernel.CloseHandle(handle)


def supervise(client, game, swap, session):
    session=Path(session).resolve(strict=True)
    client_service_host.require_client_files(client)
    # The working game loads both modules from bin64_SteamRetail, not from the
    # shadow client's private Compat folder. Verify exact DLL revisions before
    # invoking any code that may modify the installed game.
    trace = contract.verify_trace(game)
    provider = contract.verify_steam_provider(game)
    service=None
    try:
        service,shadow,modes=client_service_host.start_services(client,game,swap,broker)
        if contract.verify_trace(game) != trace or contract.verify_steam_provider(game) != provider:
            raise RuntimeError("Service startup replaced a game runtime DLL; preserving current files for inspection")
        runtime={
            "status":"ready",
            "game_exe":str(Path(game).resolve(strict=True)),
            "service_pid":service.pid,
            "shadow":str(shadow),
            "trace":str(trace),
            "steam_compat":str(provider),
            "catalog":str(contract.catalog_from_local_appdata()),
            "modes":contract.validate_modes(modes),
            # Identity, relay and overlay state must come from the real client
            # host if it exports them; do not silently inject empty values.
            "session_environment":{},
        }
        atomic_json(session/"ready.json",runtime)
        # The parent launches Evolve only AFTER receiving ready.json.
        deadline=time.monotonic()+100
        game_pid=None
        while time.monotonic()<deadline and game_pid is None:
            if (session/"cancel").exists():
                return
            if service.poll() is not None:
                raise RuntimeError("Stage2 profile or Steam compatibility host exited before game startup")
            pid_file=session/"game.pid"
            if pid_file.exists():
                raw=pid_file.read_text().strip()
                if raw.isdigit() and 1<=int(raw)<2**32:
                    game_pid=int(raw)
            time.sleep(.2)
        if game_pid is None:
            raise RuntimeError("No game was created; stopping idle services host")
        if not running_game(game_pid):
            return
        while running_game(game_pid):
            if service.poll() is not None:
                atomic_json(session/"service-failed.json",{"error":"Client services stopped before game exited"})
                # Original PAK backups stay guarded by independent watchdog.
                return
            time.sleep(1)
    except BaseException as exc:
        atomic_json(session/"error.json",{"error":str(exc)[:1200]})
        raise
    finally:
        if service is not None:
            try:
                if service.poll() is None:
                    service.terminate()
            finally:
                service.close()
        atomic_json(session/"stopped.json",{"stopped":True})


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("--client",required=True)
    parser.add_argument("--game",required=True)
    parser.add_argument("--swap",required=True)
    parser.add_argument("--session",required=True)
    args=parser.parse_args()
    try:
        supervise(args.client,args.game,args.swap,args.session)
        return 0
    except BaseException as exc:
        print("DIRECT_RUNTIME_HOST_FAILED: "+str(exc),file=sys.stderr,flush=True)
        return 1


if __name__=="__main__":
    sys.exit(main())
