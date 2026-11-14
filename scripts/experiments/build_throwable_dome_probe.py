"""Build an isolated, experimental Stage 2 mobile-arena weapon override.

This changes only the existing weapon definition. It does not change input
bindings, native fire-mode implementations, or the game's shared-dome rules.
The source XML must come from an authenticated extraction of the target PAK.
"""

import argparse
from pathlib import Path
import xml.etree.ElementTree as ET


MEMBER = Path("libs/Items/Mercs/Shared/MobileArenaTriggerWeapon.xml")


def build(source: Path, destination: Path) -> None:
    if destination.exists():
        raise ValueError(f"Refusing to overwrite existing output: {destination}")
    root = ET.parse(source).getroot()
    if root.tag != "item" or root.get("name") != "MobileArenaTriggerWeapon" or root.get("class") != "Weapon":
        raise ValueError("Source is not the Stage 2 mobile arena trigger weapon")
    firemodes = root.find("firemodes")
    params = root.find("params")
    ammos = root.find("ammos")
    if firemodes is None or params is None or ammos is None:
        raise ValueError("Expected weapon sections are missing")
    existing = firemodes.findall("firemode")
    selectable = next((p for p in params.findall("param") if p.get("name") == "selectable"), None)
    if len(existing) != 1 or existing[0].get("type") != "MobileArenaTrigger" or selectable is None or selectable.get("value") != "0":
        raise ValueError("Unexpected trigger weapon version; refusing an unreviewed override")

    selectable.set("value", "1")
    ET.SubElement(params, "param", name="itemClass", value="nw")
    geometry = ET.Element("geometry")
    ET.SubElement(geometry, "firstperson", name="characters/hunters/shared/weapons/mobilearena/mobilearena_fp.chr")
    ET.SubElement(geometry, "thirdperson", name="characters/hunters/shared/weapons/mobilearena/mobilearena_3p.chr")
    root.insert(list(root).index(ammos), geometry)
    # Keep the original shared-world entry for compatibility with any native
    # trigger references; add the existing throwable MobileArenaProjectile.
    ET.SubElement(ammos, "ammo", name="MobileArenaAmmo", extra="0", amount="1")
    firemodes.remove(existing[0])
    mode = ET.SubElement(firemodes, "firemode", type="Throw", name="Throw")
    fire = ET.SubElement(mode, "fire")
    for name, value in (("ammo_type", "MobileArenaAmmo"), ("clip_size", "-1"), ("rate", "60"),
                        ("bCanFireWhenNotSelected", "true"), ("fFireAngleOffset", "5.0")):
        ET.SubElement(fire, "param", name=name, value=value)
    throw = ET.SubElement(mode, "throw")
    ET.SubElement(throw, "param", name="fReleaseDelay", value="0.15")
    ET.SubElement(throw, "param", name="bAutoSelectLastItem", value="1")
    destination.parent.mkdir(parents=True, exist_ok=True)
    ET.ElementTree(root).write(destination, encoding="utf-8", xml_declaration=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--extracted-libs", required=True, type=Path, help="verified extracted archive root containing libs/")
    parser.add_argument("--output", required=True, type=Path, help="new, empty override folder")
    args = parser.parse_args()
    build(args.extracted_libs / MEMBER, args.output / MEMBER)
    print(f"Experimental override: {args.output / MEMBER}")
