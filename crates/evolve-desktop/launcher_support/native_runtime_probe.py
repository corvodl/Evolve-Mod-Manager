"""Verify installed native Stage2 DLL interface contracts without version hashes.

Runtime hook and service readiness still have to pass before Direct Launch is
reported successful. This module only accepts an ABI-compatible local DLL.
"""
from pathlib import Path


def inspect(path):
    data = Path(path).read_bytes()
    if not 4096 <= len(data) <= 64 * 1024 * 1024 or data[:2] != b"MZ":
        raise RuntimeError("Expected a bounded native Windows DLL")

    def at(offset, length):
        if offset < 0 or length < 0 or offset + length > len(data):
            raise RuntimeError("Truncated native runtime PE header")
        return data[offset:offset + length]

    def number(offset, length):
        return int.from_bytes(at(offset, length), "little")

    pe = number(0x3C, 4)
    if (at(pe, 4) != b"PE\x00\x00" or number(pe + 4, 2) != 0x8664
            or not number(pe + 22, 2) & 0x2000 or number(pe + 24, 2) != 0x20B):
        raise RuntimeError("Expected a 64-bit Windows DLL")

    section_count = number(pe + 6, 2)
    if not 1 <= section_count <= 32:
        raise RuntimeError("Unexpected runtime DLL sections")
    section_offset = pe + 24 + number(pe + 20, 2)
    sections = []
    for i in range(section_count):
        offset = section_offset + 40 * i
        sections.append((number(offset + 12, 4), number(offset + 16, 4),
                         number(offset + 20, 4)))

    def file_position(rva):
        for start, length, raw in sections:
            if start <= rva < start + length:
                at(raw + rva - start, 1)
                return raw + rva - start
        raise RuntimeError("Unmapped native DLL export")

    export_rva = number(pe + 24 + 112, 4)
    exports = set()
    if export_rva:
        directory = file_position(export_rva)
        names = number(directory + 24, 4)
        pointers = number(directory + 32, 4)
        if names > 4096:
            raise RuntimeError("Unexpected runtime DLL export count")
        start = file_position(pointers)
        for i in range(names):
            name_offset = file_position(number(start + i * 4, 4))
            end = data.find(b"\x00", name_offset,
                            min(name_offset + 256, len(data)))
            if end < 0:
                raise RuntimeError("Malformed native DLL export name")
            exports.add(at(name_offset, end - name_offset).decode("ascii"))
    return data, exports


def verify(path, kind):
    data, exports = inspect(path)
    if kind == "trace":
        required = (
            b"Stage2Tracer starting;",
            b"Modes from MODDED_EVOLVE_STAGE2_MODES:",
            b"ModdedEvolveSteamCompat",
        )
        if not all(marker in data for marker in required):
            raise RuntimeError("Stage2Trace runtime interface changed; use Client Launch")
    elif kind == "provider":
        required = {
            "CreateInterface", "SteamAPI_RunCallbacks",
            "Steam_CreateSteamPipe", "Steam_ConnectToGlobalUser",
        }
        if not required.issubset(exports):
            raise RuntimeError("Steam compatibility provider ABI changed; use Client Launch")
    else:
        raise ValueError("Unknown native Stage2 runtime type")
    return Path(path)
