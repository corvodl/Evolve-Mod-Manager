#!/usr/bin/env python3
"""Per-user Stage 2 services host reusing the user's installed client runtime.

Never upload or embed the client, identity store, certificates, or any profile.
The smoke entry point initializes the real services but normally quits on
--no-launch. A pinned managed-IL patch keeps this *private shadow copy* alive
while the manager owns game process creation.
"""
from __future__ import annotations
import hashlib
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time

CLIENT_DIGEST = "f933713a9ad1a670c0a83ab3fec6f3720c12d0e794d44643edc82b490a94e039"
PATCH_OFFSET = 334944
# Original Stage2Smoke.Run "--no-launch: stopping here" + return.
PATCH_EXPECTED = bytes.fromhex("72 25 56 02 70 28 33 0b 00 06 16 13 14 dd 6c 04 00 00")
# Sleep(1000) using existing MemberRef(0x0a00075d); branch back. No RVA
# changes, metadata edits, new imports, network patches, or key modifications.
PATCH_REPLACEMENT = bytes.fromhex("20 e8 03 00 00 28 5d 07 00 0a 2b f4") + bytes(6)

def digest(path):
    with open(path, "rb") as stream:
        h=hashlib.sha256()
        for chunk in iter(lambda: stream.read(1024*1024), b""):
            h.update(chunk)
        return h.hexdigest()

def patch_shadow_client(assembly):
    path=Path(assembly)
    data=bytearray(path.read_bytes())
    if hashlib.sha256(data).hexdigest() != CLIENT_DIGEST:
        raise RuntimeError("Installed Modded Evolve client version is not supported by the pinned Stage 2 services-only adapter; no client files changed.")
    if data[PATCH_OFFSET:PATCH_OFFSET+len(PATCH_EXPECTED)] != PATCH_EXPECTED:
        raise RuntimeError("Stage 2 services entry-point layout differs; refusing to modify assembly.")
    data[PATCH_OFFSET:PATCH_OFFSET+len(PATCH_EXPECTED)] = PATCH_REPLACEMENT
    path.write_bytes(data)
    if path.read_bytes()[PATCH_OFFSET:PATCH_OFFSET+18] != PATCH_REPLACEMENT:
        raise RuntimeError("Could not verify staged services-only runtime patch.")

def require_client_files(client):
    exe=Path(client).resolve(strict=True)
    if exe.name.lower() != "moddedevolvelauncher.exe" or not exe.is_file():
        raise RuntimeError("Select the original installed ModdedEvolveLauncher.exe.")
    root=exe.parent
    for name in ["ModdedEvolveLauncher.dll", "Compat/ModdedEvolveSteamCompat.dll",
                 "Compat/Stage2/Stage2Trace.dll"]:
        file=root/name
        if file.is_symlink() or not file.is_file():
            raise RuntimeError("Installed client runtime is incomplete: "+name)
    actual_digest = digest(root/"ModdedEvolveLauncher.dll")
    if actual_digest != CLIENT_DIGEST:
        raise RuntimeError(
            "Unsupported Modded Evolve service runtime version "
            f"(installed SHA-256: {actual_digest}; supported adapter SHA-256: {CLIENT_DIGEST}). "
            "The services-only adapter must be updated and validated for this client build. "
            "Use Client Launch until then; installed client files were not modified."
        )
    return root

def ensure_shadow(client_exe, swap_dir):
    root=require_client_files(client_exe)
    swap=Path(swap_dir).resolve(strict=True)
    dest=swap/"direct-client-services"
    marker=dest/"service-source-sha256.txt"
    if dest.exists():
        if dest.is_symlink() or not dest.is_dir() or not marker.is_file() or marker.read_text().strip()!=CLIENT_DIGEST:
            raise RuntimeError("Existing direct services shadow directory is unrecognized; preserve and inspect it.")
        if (dest/"ModdedEvolveLauncher.dll").read_bytes()[PATCH_OFFSET:PATCH_OFFSET+18]!=PATCH_REPLACEMENT:
            raise RuntimeError("The staged client service host has changed unexpectedly; refusing to run.")
        return dest
    work=Path(tempfile.mkdtemp(prefix=".direct-client-services-", dir=swap))
    try:
        # Copy installed code and static assets; NEVER copy AppData identities,
        # saved profiles, DPAPI credentials, session data, or runtime logs.
        shutil.copytree(root,work,dirs_exist_ok=True,
                        ignore=shutil.ignore_patterns("*.log","*.dmp","*.dpapi","*.lock"))
        patch_shadow_client(work/"ModdedEvolveLauncher.dll")
        (work/"service-source-sha256.txt").write_text(CLIENT_DIGEST+"\n")
        os.replace(work,dest)
    finally:
        if work.exists():
            shutil.rmtree(work)
    return dest

def listener_owner(port, pid):
    """Do not confuse an unrelated local server with our client host."""
    if os.name != "nt":
        return False
    outcome=subprocess.run(["netstat","-ano","-p","tcp"],capture_output=True,text=True,
                           creationflags=getattr(subprocess,"CREATE_NO_WINDOW",0),timeout=10)
    if outcome.returncode:
        return False
    for row in outcome.stdout.splitlines():
        parts=row.split()
        if len(parts)<5 or parts[3].upper()!="LISTENING":
            continue
        local=parts[1].rsplit(":",1)
        if len(local)==2 and local[1]==str(port) and parts[-1]==str(pid):
            if local[0].strip("[]") in ("127.0.0.1","0.0.0.0","::1","::"):
                return True
    return False

def wait_services(process, log_path=None, timeout=45):
    """Accept active service ownership or an explicit client runtime handshake.

    On Windows an HttpListener can expose its sockets under HTTP.sys (PID 4),
    rather than under the managed process. Requiring both listener PIDs to
    match the client can incorrectly block a working service-only test.
    """
    deadline=time.monotonic()+timeout
    while time.monotonic()<deadline:
        if process.poll() is not None:
            raise RuntimeError("Stage 2 services-only runtime exited during startup; inspect direct-stage2-services.log")
        if listener_owner(2000,process.pid) and listener_owner(2040,process.pid):
            return
        if log_path is not None:
            try:
                details=Path(log_path).read_text(encoding="utf-8", errors="replace")
            except FileNotFoundError:
                details=""
            if len(details) > 1024 * 1024:
                raise RuntimeError("Client service startup log is unexpectedly large")
            if "STAGE2_SERVICES_STARTED" in details and "SESSION_IDENTITY_STAGE2_READY" in details:
                # Explicit client-side service and identity readiness is valid
                # even if kernel-owned HTTP.sys is the reported listener PID.
                return
        time.sleep(.25)
    raise RuntimeError("Stage 2 profile/Steam service startup was not confirmed after 45 seconds; inspect direct-stage2-services.log.")

def start_services(client_exe, game_exe, swap_dir, broker):
    shadow=ensure_shadow(client_exe,swap_dir)
    log=Path(swap_dir)/"direct-stage2-services.log"
    log.write_bytes(b"")
    host=broker.launch_standard_user(
        shadow/"ModdedEvolveLauncher.exe",
        args=["--stage2-smoke",str(Path(game_exe).resolve(strict=True)),
              "--no-launch","--keep"],
        log_path=log)
    try:
        wait_services(host, log)
        modes=None
        for line in log.read_text(encoding="utf-8",errors="replace").splitlines():
            if "modes=" in line:
                modes=line.rsplit("modes=",1)[1].strip()
        if not modes or len(modes)>8192 or not all(x.isascii() for x in modes):
            raise RuntimeError("Client service host did not publish its Stage2 compatibility modes. Refusing incomplete game startup.")
        return host,shadow,modes
    except Exception:
        host.terminate()
        host.close()
        raise
