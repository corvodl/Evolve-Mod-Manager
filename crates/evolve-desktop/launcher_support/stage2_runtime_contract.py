"""Runtime contract for the October 15, 2026 Stage2Trace.dll.

This module validates files the *installed game/client* owns. It does not
provide profile credentials, impersonate an identity, copy protected client
files, or alter a game installation. Keep it independent of the launch scripts
so failures are detected before changing PAKs.
"""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import re
import stat

# Accept compatible installed game-local DLL builds by their runtime ABI and
# required hook markers, not by a frozen version hash. Startup still verifies
# the game hooks and Steam provider are active for the current session.
import native_runtime_probe as native_probe

_REQUIRED_MODES = frozenset(("Stage2Listener", "Stage2ProfileCompat", "Stage2ProfileOnly"))
_MODE_PATTERN = re.compile(r"^[A-Za-z][A-Za-z0-9_]{0,79}$")
# These are consumed by Stage2Trace and ModdedEvolveSteamCompat, respectively.
OPTIONAL_SESSION_ENV = frozenset((
    "MODDED_EVOLVE_LOCAL_IDENTITY",
    "MODDED_EVOLVE_RELAY",
    "MODDED_EVOLVE_RELAY_TOKEN",
    "MODDED_EVOLVE_RELAY_TCP_ONLY",
    "MODDED_EVOLVE_DIRECT",
    "MODDED_EVOLVE_DIRECT_BROKER",
    "MODDED_EVOLVE_DIRECT_STUN",
    "MODDED_EVOLVE_STEAMID_PROJECTION",
    "MODDED_EVOLVE_REMOTE_STORAGE",
    "MODDED_EVOLVE_JOIN_LOBBY_OF",
    "MODDED_EVOLVE_OVERLAY",
    "MODDED_EVOLVE_OVERLAY_PORT",
    "MODDED_EVOLVE_OVERLAY_INPUT",
    "MODDED_EVOLVE_VOICE",
    "MODDED_EVOLVE_IP_COUNTRY",
    "MODDED_EVOLVE_APPEARANCE_V3",
    "MODDED_EVOLVE_STAGE2_COMMUNITY_NEWS",
))


def _regular(path: Path) -> bool:
    try:
        info = path.lstat()
    except OSError:
        return False
    return stat.S_ISREG(info.st_mode) and not bool(
        getattr(info, "st_file_attributes", 0) &
        getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0)
    )


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def game_binary_directory(game_exe) -> Path:
    exe = Path(game_exe).resolve(strict=True)
    if exe.name.casefold() != "evolve.exe" or exe.parent.name.casefold() != "bin64_steamretail":
        raise RuntimeError("Expected installed bin64_SteamRetail/Evolve.exe")
    if not _regular(exe):
        raise RuntimeError("Evolve.exe must be a regular file")
    return exe.parent


def verify_trace(game_exe) -> Path:
    binary_dir = game_binary_directory(game_exe)
    path = binary_dir / "Stage2Trace.dll"
    if not _regular(path):
        raise RuntimeError("Stage2Trace.dll is missing beside Evolve.exe")
    native_probe.verify(path, "trace")
    return path


def verify_steam_provider(game_exe) -> Path:
    binary_dir = game_binary_directory(game_exe)
    path = binary_dir / "ModdedEvolveSteamCompat.dll"
    if not _regular(path):
        raise RuntimeError("Steam compatibility provider missing beside Evolve.exe")
    native_probe.verify(path, "provider")
    return path


def catalog_from_local_appdata(local_appdata=None) -> Path:
    base = local_appdata if local_appdata is not None else os.environ.get("LOCALAPPDATA")
    if not base:
        raise RuntimeError("LOCALAPPDATA is missing; cannot find the client's ownable catalog")
    path = Path(base) / "ModdedEvolveLauncher" / "stage2" / "ownable-catalog-launch.txt"
    if not _regular(path):
        raise RuntimeError(
            "The live client has not generated stage2/ownable-catalog-launch.txt. "
            "A directory or a substituted sample catalog is not sufficient."
        )
    size = path.stat().st_size
    if size <= 0 or size > 16 * 1024 * 1024:
        raise RuntimeError("Client's Stage 2 ownable catalog is empty or oversized")
    return path.resolve(strict=True)


def validate_modes(raw: str) -> str:
    if not isinstance(raw, str) or not raw or len(raw) > 4096:
        raise RuntimeError("The client did not provide a valid Stage2 modes string")
    modes = raw.split(",")
    if any(not _MODE_PATTERN.fullmatch(item) for item in modes):
        raise RuntimeError("Client's Stage2 modes are malformed")
    if not _REQUIRED_MODES.issubset(set(modes)):
        raise RuntimeError("Client's Stage2 compatibility modes omit profile or listener support")
    if len(modes) != len(set(modes)):
        raise RuntimeError("Client's Stage2 compatibility modes contain duplicates")
    return raw


def validated_optional_session_environment(raw) -> dict[str, str]:
    """Only forward actual session values; never synthesize an identity or relay."""
    if raw is None:
        return {}
    if not isinstance(raw, dict):
        raise RuntimeError("The client session environment must be an object")
    result = {}
    for key, value in raw.items():
        if key not in OPTIONAL_SESSION_ENV or not isinstance(value, str):
            raise RuntimeError("Unrecognized Stage2/Steam compatibility session field")
        if not value or len(value) > 4096 or "\0" in value or "\n" in value:
            raise RuntimeError("An optional client session value is empty or malformed")
        result[key] = value
    return result


def build_game_environment(state: dict, swap_dir) -> dict[str, str]:
    """Construct game-side variables only from an *active verified* runtime."""
    if not isinstance(state, dict) or state.get("status") != "ready":
        raise RuntimeError("Stage2 service host has not completed runtime preparation")
    trace = verify_trace(state["game_exe"])
    provider = verify_steam_provider(state["game_exe"])
    catalog = catalog_from_local_appdata(state.get("local_appdata"))
    if Path(state["trace"]).resolve(strict=True) != trace:
        raise RuntimeError("Supervisor requested a different Stage2Trace.dll")
    if Path(state["steam_compat"]).resolve(strict=True) != provider:
        raise RuntimeError("Supervisor requested a different Steam compatibility provider")
    env = validated_optional_session_environment(state.get("session_environment"))
    env.update({
        "MODDED_EVOLVE_GAME": "stage2",
        "MODDED_EVOLVE_STAGE2_FRONTEND": "127.0.0.1:2000",
        "MODDED_EVOLVE_STAGE2_MODES": validate_modes(state["modes"]),
        "MODDED_EVOLVE_STAGE2_CATALOG": str(catalog),
        "MODDED_EVOLVE_STAGE2_LOG": str(Path(swap_dir) / "rust-direct-stage2-hook.log"),
        "MODDED_EVOLVE_STEAM_EMU_DLL": str(provider),
        "MODDED_EVOLVE_COMPAT_TELEMETRY_PATH": str(Path(swap_dir) / "rust-direct-provider-telemetry.json"),
    })
    return env


def validate_hook_startup(log_text: str, provider: Path, modes: str) -> None:
    """Check hook-side evidence, not just a successful remote LoadLibrary."""
    if not isinstance(log_text, str) or len(log_text) > 1024 * 1024:
        raise RuntimeError("Stage2 hook startup log is missing or oversized")
    if "Stage2Tracer starting;" not in log_text:
        raise RuntimeError("Stage2Trace did not report startup")
    if "Hooks enabled status=MH_OK" not in log_text:
        raise RuntimeError("Stage2Trace failed to enable its game hooks")
    if f"STEAM_EMU active provider={provider}" not in log_text:
        raise RuntimeError("Game did not activate the verified Steam compatibility provider")
    expected_line = "Modes from MODDED_EVOLVE_STAGE2_MODES: " + validate_modes(modes)
    if expected_line not in log_text:
        raise RuntimeError("Stage2Trace did not activate the client's intended mode set")


def wait_for_hook_startup(log_path: Path, provider: Path, modes: str, process, timeout=20) -> None:
    """Require positive hook log evidence after process creation/resume.

    The log must be truncated *before* launching the child. A stale previous
    success is not proof of the current process's startup.
    """
    import time
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("Evolve.exe exited before Stage2Trace/SteamCompat startup completed")
        try:
            content = Path(log_path).read_text(encoding="utf-8", errors="replace")
        except FileNotFoundError:
            content = ""
        if len(content) > 1024 * 1024:
            raise RuntimeError("Stage2 hook log exceeded the startup size limit")
        try:
            validate_hook_startup(content, provider, modes)
        except RuntimeError:
            time.sleep(0.2)
            continue
        return
    raise RuntimeError(
        "Stage2Trace did not confirm hook startup and the verified Steam provider "
        "within the startup deadline. Inspect the direct Stage2 hook log."
    )
