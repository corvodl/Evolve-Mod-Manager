"""Generate Legacy PreRound's AI_* arrays from Stage 2's ability XML assets."""
import json
import os
import pickle
import re
import sys
import xml.etree.ElementTree as ET

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GROUPS = {
    "goliatha": "goliath",
    "goliathmeteor": "goliathmeteor",
    "groundera": "grounder",
    "krakenelder": "krakenelder",
    "lurkera": "lurker",
    "rockya": "rocky",
    "rockybobcicle": "rockybobcicle",
    "teleportera": "teleporter",
    "wraithphantom": "wraithphantom",
}


def levels(root, params):
    if params.get("level"):
        return [int(params["level"])]
    return list(range(1, int(root.get("levels", "1")) + 1))


def at(params, key, level):
    values = params.get(key, "").split("|")
    return values[min(level - 1, len(values) - 1)]


def build():
    index = pickle.load(open(os.path.join(ROOT, "tools", "index.pkl"), "rb"))
    result = {}
    for subtype, group in GROUPS.items():
        slots = {}
        for path, sources in sorted(index.items()):
            if not (path.startswith("libs/abilities/" + group + "/") and path.endswith(".xml")):
                continue
            src = next((item for item in sources if item[0] == "S2"), None)
            if src is None:
                continue
            ability = ET.parse(os.path.join(src[1], "files", src[2])).getroot()
            params = {p.get("name"): p.get("value", "") for p in ability.findall("./params/param")}
            if not params.get("ability_slot"):
                continue
            slot = int(params["ability_slot"])
            # Grounder's LightningDome is a duplicate/unused ChainLightning
            # prototype occupying PlasmaBlast's fourth slot.
            if group == "grounder" and ability.get("name") == "GrounderLightningDome":
                continue
            base = re.sub(r"[123]$", "", ability.get("name"))
            for level in levels(ability, params):
                record = (base, params.get("display_name", ""),
                          params.get("tutorial_video", ""),
                          at(params, "display_desc", level),
                          at(params, "stat1_desc", level),
                          at(params, "stat2_desc", level))
                if (slot, level) in slots and slots[slot, level] != record:
                    # LurkerMinionDDLurkerAcidBarrage is a minion variant,
                    # not the player's AcidBarrage in slot three.
                    if "MinionDD" in ability.get("name", ""):
                        continue
                    raise ValueError((subtype, slot, level, ability.get("name")))
                slots[slot, level] = record
        expected = {(slot, level) for slot in range(1, 5) for level in range(1, 4)}
        if set(slots) != expected:
            raise ValueError((subtype, "missing", sorted(expected - set(slots))))
        values = []
        for slot in range(1, 5):
            for level in range(1, 4):
                base, name, video, desc, stat1, stat2 = slots[slot, level]
                if not all((base, name, video, desc)):
                    raise ValueError((subtype, slot, level, "incomplete"))
                values.extend((base, base, name, slot, video, False, level, desc, stat1, stat2))
        result[subtype] = values
    return result


def main():
    mapping = build()
    lines = ["function s2_abilityInfoForMonster(subtype)", "{",
             "   var key = String(subtype).toLowerCase();",
             "   while(key.length > 0 && key.charAt(key.length - 1) >= \"0\" && key.charAt(key.length - 1) <= \"9\")",
             "   {",
             "      key = key.substr(0,key.length - 1);",
             "   }",
             "   switch(key)", "   {"]
    for key, values in mapping.items():
        def as2(x):
            return ("true" if x else "false") if isinstance(x, bool) else str(x) if isinstance(x, int) else json.dumps(x)
        lines.extend(("      case " + json.dumps(key) + ":",
                      "         return new Array(" + ",".join(map(as2, values)) + ");"))
    lines.extend(("      default:", "         return new Array();", "   }", "}"))
    with open(sys.argv[1], "w", encoding="utf-8") as out:
        out.write("\n".join(lines) + "\n")
    print("Generated", len(mapping), "monster ability sets, 4 slots x 3 levels each")


if __name__ == "__main__":
    main()
