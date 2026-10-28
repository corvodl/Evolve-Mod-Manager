#!/usr/bin/env python3
"""Read-only, conservative inventory of an extracted signed CharacterStage.

Inputs must be scratch extractions, not the installed game or manager stage.
An unresolved reference is *not* proof that it is missing from the game: only
explicitly supplied authenticated inventory reports are compared.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from xml.etree import ElementTree

MAX_ITEMS = 100_000
RESOURCE = re.compile(r"(?i)\.(?:cdf|chr|cgf|skin|mtl|dds|usm|gfx|xml|pak|lua)$")


def member(name: str) -> str | None:
    name = name.strip().replace("\\", "/")
    if (not name or len(name) > 2048 or name.startswith("/") or ":" in name
            or any(part in ("", ".", "..") for part in name.split("/"))):
        return None
    return name.lower()


def read_inventory(report: Path) -> set[str]:
    data = json.loads(report.read_text(encoding="utf-8"))
    entries = data.get("entries")
    if not isinstance(entries, list) or len(entries) > MAX_ITEMS:
        raise ValueError(f"Invalid inventory in {report}")
    result = set()
    for entry in entries:
        path = entry.get("path") if isinstance(entry, dict) else None
        normalized = member(path) if isinstance(path, str) else None
        if normalized is None:
            raise ValueError(f"Unsafe inventory path in {report}")
        result.add(normalized)
    return result


def extracted_paths(root: Path) -> set[str]:
    result = set()
    for file in root.rglob("*"):
        if file.is_symlink():
            raise ValueError(f"Symlink in extraction: {file}")
        if file.is_file():
            normalized = member(file.relative_to(root).as_posix())
            if normalized is None or len(result) >= MAX_ITEMS:
                raise ValueError("Invalid or oversized extracted member inventory")
            result.add(normalized)
    return result


def references(level: Path, level_data: Path) -> set[str]:
    resources = level / "resourcelist.txt"
    if not resources.is_file() or resources.is_symlink() or resources.stat().st_size > 1024 * 1024:
        raise ValueError("Missing or oversized extracted resourcelist.txt")
    found = set()
    for line in resources.read_text(encoding="utf-8-sig").splitlines():
        path = member(line)
        if path and RESOURCE.search(path):
            found.add(path)
    # External-layer XML can identify additional model/material paths absent
    # from the engine's broad resourcelist. Do not expand entities or XML DTDs.
    for root in (level, level_data):
        for file in root.rglob("*.xml"):
            if file.is_symlink() or file.stat().st_size > 8 * 1024 * 1024:
                raise ValueError(f"Unsafe or oversized XML: {file}")
            raw = file.read_bytes()
            if b"<!DOCTYPE" in raw.upper() or b"<!ENTITY" in raw.upper():
                raise ValueError(f"XML external entities are forbidden: {file}")
            for element in ElementTree.fromstring(raw).iter():
                for value in element.attrib.values():
                    path = member(value)
                    if path and RESOURCE.search(path):
                        found.add(path)
    if len(found) > MAX_ITEMS:
        raise ValueError("Too many CharacterStage resource references")
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("level", type=Path, help="scratch extraction of CharacterStage/level.pak")
    parser.add_argument("level_data", type=Path, help="scratch extraction of Level_UI_Menu_Levels_data.pak")
    parser.add_argument("--inventory", type=Path, action="append", default=[],
                        help="authenticated signed-PAK inventory JSON; repeat as needed")
    args = parser.parse_args()
    try:
        if not args.level.is_dir() or not args.level_data.is_dir():
            raise ValueError("Both scratch extraction directories are required")
        extracted = extracted_paths(args.level) | extracted_paths(args.level_data)
        known = set().union(*(read_inventory(path) for path in args.inventory))
        found = references(args.level, args.level_data)
        print(json.dumps({
            "references": len(found),
            "in_extractions": sorted(found & extracted),
            "in_supplied_inventories": sorted((found - extracted) & known),
            "unresolved_not_proven_missing": sorted(found - extracted - known),
            "caution": "No inference about mounting new PAK paths or unlisted game archives",
        }, indent=2))
        return 0
    except (OSError, ValueError, ElementTree.ParseError) as error:
        print(f"CharacterStage audit stopped: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
