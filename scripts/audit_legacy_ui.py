#!/usr/bin/env python3
"""Read-only Legacy Evolve UI Data and UI Assets reference audit.

This tool does not rebuild or patch GFX, copy copyrighted assets into the
repository, or install the Legacy screen schema into Stage 2. It records
only declarations needed to plan Stage 2-owned visual changes.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys
from xml.etree import ElementTree as ET
from zipfile import ZipFile

DATA_ROOT = "files/Libs/UI/"
MAIN_SCREEN = DATA_ROOT + "MainMenuScreen.xml"
MAIN_UI = DATA_ROOT + "UIElements/MainMenu.xml"
NEWS = DATA_ROOT + "MainMenuNews.xml"
MAX_XML = 256 * 1024
MAX_MEMBERS = 20_000

VISUAL_ASSETS = (
    "Libs/UI/main_menu.gfx",
    "Libs/UI/main_menu_I19.dds.0",
    "Libs/UI/main_menu_I70.dds.0",
    "Libs/UI/main_menu_ID.dds.0",
    "Libs/UI/gfxfontlib.gfx",
    "Libs/UI/gfxfontlib_glyphs.gfx",
    "Libs/UI/FlashAssets/images/News/news_img1.dds.0",
    "Libs/UI/FlashAssets/images/News/news_img2.dds.0",
    "Libs/UI/FlashAssets/images/News/news_img3.dds.0",
)


def read_xml(archive: ZipFile, member: str) -> ET.Element:
    info = archive.getinfo(member)
    if info.file_size > MAX_XML:
        raise ValueError("Oversized UI Data XML: " + member)
    with archive.open(info) as source:
        xml = source.read(MAX_XML + 1)
    if len(xml) > MAX_XML or b"<!DOCTYPE" in xml.upper() or b"<!ENTITY" in xml.upper():
        raise ValueError("Unbounded or unsupported UI Data XML: " + member)
    return ET.fromstring(xml)


def archive_names(archive: ZipFile) -> set[str]:
    names = archive.namelist()
    if len(names) > MAX_MEMBERS:
        raise ValueError("Too many archive members")
    if len(names) != len(set(names)):
        raise ValueError("Duplicate archive member names")
    return set(names)


def audit(ui_data: Path, assets: Path | None = None) -> dict:
    with ZipFile(ui_data) as archive:
        names = archive_names(archive)
        if not {MAIN_SCREEN, MAIN_UI, NEWS}.issubset(names):
            raise ValueError("Missing required Legacy main menu UI Data XML")
        screen = read_xml(archive, MAIN_SCREEN)
        ui = read_xml(archive, MAIN_UI)
        news = read_xml(archive, NEWS)

    element = ui.find(".//UIElement[@name='MainMenuNew']")
    if element is None:
        raise ValueError("Legacy MainMenuNew UI element not declared")
    gfx = element.find("GFx")
    if gfx is None or gfx.get("file") != "main_menu.gfx":
        raise ValueError("Expected Legacy main_menu.gfx binding missing")
    main = screen.find("./MainMenu/Main")
    if main is None:
        raise ValueError("Legacy Main button declarations missing")
    pc = []
    for button in main.findall("Button"):
        # Preserve original declarations and condition names. Platform-less
        # entries and explicitly PC entries are the PC visual reference;
        # they are not Stage 2 executable commands.
        if button.get("platform", "pc") != "pc":
            continue
        pc.append({
            key: button.get(key, "")
            for key in (
                "display_name", "command", "platform", "cinematic_event",
                "hidden_when", "disabled_when", "filter",
            )
        })
    declared = {
        "movie": gfx.get("file"),
        "layer": gfx.get("layer"),
        "input": {k: element.get(k, "") for k in (
            "mouseevents", "cursor", "controller_input",
        )},
        "dynamic_arrays": [
            arr.get("varname", "") for arr in element.findall("./arrays/array")
        ],
        "native_functions": [
            fn.get("funcname", "") for fn in element.findall("./functions/function")
        ],
    }
    news_items = []
    for entry in news.findall("./News"):
        news_items.append({
            "image": entry.get("img", ""),
            "has_text": bool(entry.get("text")),
        })
    result = {
        "intent": "Legacy visual reference only; keep Stage 2 runtime UI Data, scripts and actions",
        "legacy_main_menu_pc_buttons": pc,
        "legacy_main_menu_ui_binding": declared,
        "legacy_news": news_items,
    }
    if assets is not None:
        with ZipFile(assets) as archive:
            names = archive_names(archive)
            result["asset_inventory"] = {
                asset: ("files/" + asset) in names for asset in VISUAL_ASSETS
            }
            result["news_reference_images"] = {
                item["image"]: ("files/" + item["image"] + ".0") in names
                for item in news_items
            }
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("ui_data", type=Path, help="Legacy UI Data ZIP (external to repository)")
    parser.add_argument("--assets", type=Path, help="Optional assembled Legacy UI Assets ZIP")
    args = parser.parse_args()
    try:
        report = audit(args.ui_data, args.assets)
    except (OSError, ValueError, ET.ParseError, KeyError) as exc:
        print(f"Legacy UI audit failed: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
