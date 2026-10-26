#!/usr/bin/env python3
"""Start the normal Modded Evolve launcher with the interactive user's medium
integrity token, even when the Mod Manager / Frida swap helper is elevated.

No credentials, UAC prompts, passwords, Explorer shell commands, injected DLLs
or remote authentication modifications. Fail closed if the interactive shell
cannot provide a verified non-elevated primary token.
"""
from __future__ import annotations

import argparse
import ctypes
from ctypes import wintypes
import os
from pathlib import Path
import subprocess
import sys


PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
TOKEN_QUERY = 0x0008
TOKEN_DUPLICATE = 0x0002
TOKEN_ASSIGN_PRIMARY = 0x0001
TOKEN_ADJUST_DEFAULT = 0x0080
TOKEN_ADJUST_SESSIONID = 0x0100
SE_IMPERSONATION = 2
TOKEN_PRIMARY = 1
LOGON_WITH_PROFILE = 0x00000001
CREATE_UNICODE_ENVIRONMENT = 0x00000400
STILL_ACTIVE = 259
TOKEN_SESSION_ID = 12
TOKEN_ELEVATION = 20
TOKEN_INTEGRITY_LEVEL = 25
SECURITY_MANDATORY_MEDIUM_RID = 0x2000


class STARTUPINFOW(ctypes.Structure):
    _fields_ = [
        ("cb", wintypes.DWORD),
        ("lpReserved", wintypes.LPWSTR),
        ("lpDesktop", wintypes.LPWSTR),
        ("lpTitle", wintypes.LPWSTR),
        ("dwX", wintypes.DWORD),
        ("dwY", wintypes.DWORD),
        ("dwXSize", wintypes.DWORD),
        ("dwYSize", wintypes.DWORD),
        ("dwXCountChars", wintypes.DWORD),
        ("dwYCountChars", wintypes.DWORD),
        ("dwFillAttribute", wintypes.DWORD),
        ("dwFlags", wintypes.DWORD),
        ("wShowWindow", wintypes.WORD),
        ("cbReserved2", wintypes.WORD),
        ("lpReserved2", ctypes.c_void_p),
        ("hStdInput", wintypes.HANDLE),
        ("hStdOutput", wintypes.HANDLE),
        ("hStdError", wintypes.HANDLE),
    ]


class PROCESS_INFORMATION(ctypes.Structure):
    _fields_ = [
        ("hProcess", wintypes.HANDLE),
        ("hThread", wintypes.HANDLE),
        ("dwProcessId", wintypes.DWORD),
        ("dwThreadId", wintypes.DWORD),
    ]


class SID_AND_ATTRIBUTES(ctypes.Structure):
    _fields_ = [("Sid", ctypes.c_void_p), ("Attributes", wintypes.DWORD)]


def _win():
    if os.name != "nt":
        raise RuntimeError("Standard-user launcher requires Windows")
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    user = ctypes.WinDLL("user32", use_last_error=True)
    advapi = ctypes.WinDLL("advapi32", use_last_error=True)
    userenv = ctypes.WinDLL("userenv", use_last_error=True)

    def bind(dll, name, restype, argtypes):
        f = getattr(dll, name)
        f.restype = restype
        f.argtypes = argtypes
        return f

    H = wintypes.HANDLE
    D = wintypes.DWORD
    P = ctypes.POINTER
    api = {
        "GetCurrentProcess": bind(kernel, "GetCurrentProcess", H, []),
        "GetShellWindow": bind(user, "GetShellWindow", wintypes.HWND, []),
        "GetWindowThreadProcessId": bind(user, "GetWindowThreadProcessId", D, [wintypes.HWND, P(D)]),
        "OpenProcess": bind(kernel, "OpenProcess", H, [D, wintypes.BOOL, D]),
        "CloseHandle": bind(kernel, "CloseHandle", wintypes.BOOL, [H]),
        "QueryFullProcessImageNameW": bind(kernel, "QueryFullProcessImageNameW", wintypes.BOOL, [H, D, wintypes.LPWSTR, P(D)]),
        "OpenProcessToken": bind(advapi, "OpenProcessToken", wintypes.BOOL, [H, D, P(H)]),
        "GetTokenInformation": bind(advapi, "GetTokenInformation", wintypes.BOOL, [H, D, ctypes.c_void_p, D, P(D)]),
        "GetSidSubAuthorityCount": bind(advapi, "GetSidSubAuthorityCount", P(ctypes.c_ubyte), [ctypes.c_void_p]),
        "GetSidSubAuthority": bind(advapi, "GetSidSubAuthority", P(D), [ctypes.c_void_p, D]),
        "DuplicateTokenEx": bind(advapi, "DuplicateTokenEx", wintypes.BOOL, [H, D, ctypes.c_void_p, D, D, P(H)]),
        "CreateEnvironmentBlock": bind(userenv, "CreateEnvironmentBlock", wintypes.BOOL, [P(ctypes.c_void_p), H, wintypes.BOOL]),
        "DestroyEnvironmentBlock": bind(userenv, "DestroyEnvironmentBlock", wintypes.BOOL, [ctypes.c_void_p]),
        "CreateProcessWithTokenW": bind(advapi, "CreateProcessWithTokenW", wintypes.BOOL,
            [H, D, wintypes.LPCWSTR, wintypes.LPWSTR, D, ctypes.c_void_p, wintypes.LPCWSTR,
             P(STARTUPINFOW), P(PROCESS_INFORMATION)]),
        "GetExitCodeProcess": bind(kernel, "GetExitCodeProcess", wintypes.BOOL, [H, P(D)]),
        "TerminateProcess": bind(kernel, "TerminateProcess", wintypes.BOOL, [H, D]),
    }
    return api


def _error(what):
    return RuntimeError(f"{what} failed (Windows error {ctypes.get_last_error()})")


def _token_data(api, token, kind):
    length = wintypes.DWORD()
    api["GetTokenInformation"](token, kind, None, 0, ctypes.byref(length))
    if not 4 <= length.value <= 1024:
        raise _error("GetTokenInformation size")
    buffer = ctypes.create_string_buffer(length.value)
    if not api["GetTokenInformation"](token, kind, buffer, length, ctypes.byref(length)):
        raise _error("GetTokenInformation")
    return buffer


def _token_dword(api, token, kind):
    buf = _token_data(api, token, kind)
    return ctypes.cast(buf, ctypes.POINTER(wintypes.DWORD)).contents.value


def _token_integrity(api, token):
    buf = _token_data(api, token, TOKEN_INTEGRITY_LEVEL)
    attrs = ctypes.cast(buf, ctypes.POINTER(SID_AND_ATTRIBUTES)).contents
    count = api["GetSidSubAuthorityCount"](attrs.Sid)
    if not count:
        raise _error("GetSidSubAuthorityCount")
    rid = api["GetSidSubAuthority"](attrs.Sid, count[0] - 1)
    if not rid:
        raise _error("GetSidSubAuthority")
    return rid[0]


def _get_process_token(api, pid=None):
    H = wintypes.HANDLE
    p = None
    if pid is None:
        p = api["GetCurrentProcess"]()  # pseudo handle; do not close
    else:
        p = api["OpenProcess"](PROCESS_QUERY_LIMITED_INFORMATION, False, int(pid))
        if not p:
            raise _error("OpenProcess")
    try:
        token = H()
        if not api["OpenProcessToken"](p, TOKEN_QUERY | TOKEN_DUPLICATE, ctypes.byref(token)):
            raise _error("OpenProcessToken")
        return token
    finally:
        if pid is not None:
            api["CloseHandle"](p)


def get_elevation(pid=None):
    """Read an actual process token; never infer privileges from a parent PID."""
    api = _win()
    token = _get_process_token(api, pid)
    try:
        return {
            "elevated": bool(_token_dword(api, token, TOKEN_ELEVATION)),
            "session": _token_dword(api, token, TOKEN_SESSION_ID),
            "integrity_rid": _token_integrity(api, token),
        }
    finally:
        api["CloseHandle"](token)


class StandardUserProcess:
    """Small Popen-like wrapper owning the process handle supplied by Win32."""
    def __init__(self, api, pid, handle):
        self._api = api
        self.pid = int(pid)
        self._handle = handle

    def poll(self):
        status = wintypes.DWORD()
        if not self._api["GetExitCodeProcess"](self._handle, ctypes.byref(status)):
            raise _error("GetExitCodeProcess")
        return None if status.value == STILL_ACTIVE else int(status.value)

    def terminate(self):
        if self.poll() is None and not self._api["TerminateProcess"](self._handle, 1):
            raise _error("TerminateProcess")

    def close(self):
        if self._handle:
            self._api["CloseHandle"](self._handle)
            self._handle = None

    def __del__(self):
        if getattr(self, "_handle", None):
            self.close()


def launch_standard_user(executable):
    """Use the signed-in Explorer desktop token and CreateProcessWithTokenW.

    Requires an accessible, MEDIUM-integrity non-elevated Explorer shell in
    the same interactive session. If absent, fail; never fall back to admin.
    """
    api = _win()
    exe = Path(executable).resolve(strict=True)
    if not exe.is_file() or exe.name.casefold() != "moddedevolvelauncher.exe":
        raise ValueError("Only a configured ModdedEvolveLauncher.exe may be launched")
    shell = api["GetShellWindow"]()
    if not shell:
        raise RuntimeError("Interactive Explorer shell not found; standard-user launch unavailable")
    shell_pid = wintypes.DWORD()
    api["GetWindowThreadProcessId"](shell, ctypes.byref(shell_pid))
    if not shell_pid.value:
        raise _error("GetWindowThreadProcessId")
    shell_process = api["OpenProcess"](PROCESS_QUERY_LIMITED_INFORMATION, False, shell_pid)
    if not shell_process:
        raise _error("OpenProcess(Explorer)")
    try:
        image = ctypes.create_unicode_buffer(32768)
        length = wintypes.DWORD(len(image))
        if not api["QueryFullProcessImageNameW"](shell_process, 0, image, ctypes.byref(length)):
            raise _error("QueryFullProcessImageNameW")
        if Path(image.value).name.casefold() != "explorer.exe":
            raise RuntimeError("Interactive shell is not Windows Explorer")
    finally:
        api["CloseHandle"](shell_process)

    shell_token = _get_process_token(api, shell_pid.value)
    try:
        own_token = _get_process_token(api)
        try:
            current_session = _token_dword(api, own_token, TOKEN_SESSION_ID)
        finally:
            api["CloseHandle"](own_token)
        shell_session = _token_dword(api, shell_token, TOKEN_SESSION_ID)
        if shell_session != current_session:
            raise RuntimeError("Explorer belongs to a different Windows logon session")
        if _token_dword(api, shell_token, TOKEN_ELEVATION) != 0:
            raise RuntimeError("Explorer is elevated; refusing to launch Evolve as administrator")
        if _token_integrity(api, shell_token) != SECURITY_MANDATORY_MEDIUM_RID:
            raise RuntimeError("Explorer token does not have medium integrity")

        primary = wintypes.HANDLE()
        access = TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT | TOKEN_ADJUST_SESSIONID
        if not api["DuplicateTokenEx"](shell_token, access, None, SE_IMPERSONATION,
                                         TOKEN_PRIMARY, ctypes.byref(primary)):
            raise _error("DuplicateTokenEx")
    finally:
        api["CloseHandle"](shell_token)
    try:
        env = ctypes.c_void_p()
        if not api["CreateEnvironmentBlock"](ctypes.byref(env), primary, False):
            raise _error("CreateEnvironmentBlock")
        try:
            startup = STARTUPINFOW()
            startup.cb = ctypes.sizeof(startup)
            startup.lpDesktop = "winsta0\\default"
            info = PROCESS_INFORMATION()
            command = ctypes.create_unicode_buffer(subprocess.list2cmdline([str(exe)]))
            if not api["CreateProcessWithTokenW"](
                primary, LOGON_WITH_PROFILE, str(exe), command, CREATE_UNICODE_ENVIRONMENT,
                env, str(exe.parent), ctypes.byref(startup), ctypes.byref(info)
            ):
                raise _error("CreateProcessWithTokenW")
        finally:
            api["DestroyEnvironmentBlock"](env)
    finally:
        api["CloseHandle"](primary)

    api["CloseHandle"](info.hThread)
    proc = StandardUserProcess(api, info.dwProcessId, info.hProcess)
    try:
        actual = get_elevation(proc.pid)
        if actual["elevated"] or actual["integrity_rid"] != SECURITY_MANDATORY_MEDIUM_RID:
            proc.terminate()
            raise RuntimeError("Client did not start as a verified standard-user process")
        if actual["session"] != shell_session:
            proc.terminate()
            raise RuntimeError("Client started in the wrong interactive session")
    except Exception:
        proc.close()
        raise
    return proc


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("--launch", metavar="CLIENT_EXE")
    parser.add_argument("--diagnose-pid", type=int, metavar="PID")
    args = parser.parse_args(argv)
    if bool(args.launch) == (args.diagnose_pid is not None):
        parser.error("Pass either --launch EXE or --diagnose-pid PID")
    if args.launch:
        process = launch_standard_user(args.launch)
        try:
            actual = get_elevation(process.pid)
            print(
                f"STANDARD_USER_LAUNCH pid={process.pid} elevated=false "
                f"integrity=medium session={actual['session']}",
                flush=True,
            )
        finally:
            process.close()  # Does not terminate running client.
    else:
        info = get_elevation(args.diagnose_pid)
        print(
            f"PROCESS_PRIVILEGES pid={args.diagnose_pid} "
            f"elevated={str(info['elevated']).lower()} "
            f"integrity_rid=0x{info['integrity_rid']:x} session={info['session']}",
            flush=True,
        )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError) as error:
        print(f"STANDARD_USER_LAUNCH_FAILED: {error}", file=sys.stderr, flush=True)
        sys.exit(1)
