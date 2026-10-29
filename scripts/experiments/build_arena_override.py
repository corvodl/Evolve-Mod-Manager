#!/usr/bin/env python3
"""Create a reversible ArenaMode.xml smoke-test override from a user-owned libs ZIP.

This does not create a second monster, change player ownership or sign a PAK.
Import the XML through the manager's isolated mod/rebuild workflow.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import xml.etree.ElementTree as ET
from zipfile import ZipFile

MEMBER = "libs/GameRules/ArenaMode.xml"
EXPECTED = {
    "nBattleWinsRequired": "2",
    "BattleDrawTime": "6:00",
    "nHoldInPlaceTime": "5",
    "nNoFiringTime": "3",
}
SMOKE = {
    "nBattleWinsRequired": "1",
    "BattleDrawTime": "5:00",
    "nHoldInPlaceTime": "3",
    "nNoFiringTime": "2",
}

def make_override(original: bytes) -> bytes:
    root = ET.fromstring(original)
    if root.tag != "ScenarioData":
        raise ValueError("Expected stock ScenarioData")
    arena, round_end = root.find("ArenaMode"), root.find("RoundEnd")
    if arena is None or round_end is None:
        raise ValueError("Missing ArenaMode or RoundEnd")
    if any(arena.get(k) != v for k, v in EXPECTED.items()):
        raise ValueError("Unknown Arena configuration; refusing to patch")
    if round_end.get("DisableMonsterAliveCheck") != "true":
        raise ValueError("Unknown Arena win condition")
    for key, value in SMOKE.items():
        arena.set(key, value)
    return ET.tostring(root, encoding="utf-8")

def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--libs-zip", required=True, type=Path)
    p.add_argument("--output", required=True, type=Path)
    args = p.parse_args()
    source = args.libs_zip.resolve(strict=True)
    output = args.output.resolve()
    if output.exists() and (not output.is_dir() or any(output.iterdir())):
        p.error("Output must be a new or empty scratch directory")
    with ZipFile(source) as archive:
        entries = [i for i in archive.infolist() if i.filename == MEMBER]
        if len(entries) != 1 or entries[0].file_size > 32768:
            raise ValueError("Expected exactly one bounded ArenaMode.xml")
        original = archive.read(entries[0])
    patched = make_override(original)
    path = output / MEMBER
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("xb") as f:
        f.write(patched)
    manifest = {
        "member": MEMBER,
        "target_pak": "libs.pak",
        "status": "Arena settings smoke-test, NOT monster-vs-monster gameplay",
        "original_sha256": hashlib.sha256(original).hexdigest(),
        "patched_sha256": hashlib.sha256(patched).hexdigest(),
        "changes": SMOKE,
        "note": "Use the mod manager's isolated signed-PAK workflow; do not overwrite stock game files.",
    }
    (output / "experiment.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(manifest, indent=2))

if __name__ == "__main__":
    main()
