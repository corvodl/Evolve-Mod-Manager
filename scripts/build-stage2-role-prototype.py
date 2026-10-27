#!/usr/bin/env python3
"""Build an isolated Stage 2 role-picker interaction prototype from a user's GFX.

No game assets or signing keys are included here. Never writes to a game or
manager stage; use a separate output folder and sign the resulting member with
the manager's isolated signed-PAK overlay workflow before testing.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path


def once(text: str, old: str, new: str) -> str:
    count = text.count(old)
    if count != 1:
        raise ValueError(f"Expected one stock role-picker anchor, found {count}: {old[:75]!r}")
    return text.replace(old, new, 1)


def patch_script(text: str) -> str:
    """Keep Stage 2's class selection, lock state and native callbacks."""
    # FFDec 26 indents nested ActionScript with three spaces. Normalize only
    # whitespace so the exact, guarded anchors below are version-independent.
    text = text.replace("\n   ", "\n    ")
    text = once(
        text,
        '    mLastSelectedClass = lastSelectedClass;\n',
        '    mLastSelectedClass = lastSelectedClass;\n'
        '    mLegacyRoleStep = 0;\n'
        '    mLegacyRoleStates = new Object();\n',
    )
    text = once(
        text,
        '    mcTitle.text = "@ui_rp_choose_role";',
        '    mcTitle.text = "SELECT YOUR ROLE";',
    )
    text = once(
        text,
        '    mRoleButtonData = [_loc26_,_loc22_,_loc23_,_loc24_,_loc25_];\n'
        '    if(mIsObserverModeEnabled == true)\n'
        '    {\n'
        '       mRoleButtonData.push(_loc27_);\n'
        '    }\n'
        '    mcList.clearList();',
        '    if(mLegacyRoleStep == 1)\n'
        '    {\n'
        '       mRoleButtonData = [_loc22_,_loc23_,_loc24_,_loc25_];\n'
        '       mcTitle.text = "SELECT YOUR CLASS";\n'
        '    }\n'
        '    else\n'
        '    {\n'
        '       // Reuse the native Assault icon as the Hunter card; this does\n'
        '       // not send Assault to the host until the player picks a class.\n'
        '       _loc22_.displayName = "HUNTER";\n'
        '       _loc22_.legacyHunterCard = true;\n'
        '       mRoleButtonData = [_loc22_,_loc26_];\n'
        '       if(mIsObserverModeEnabled == true)\n'
        '       {\n'
        '          mRoleButtonData.push(_loc27_);\n'
        '       }\n'
        '       mcTitle.text = "SELECT YOUR ROLE";\n'
        '    }\n'
        '    var oldButton;\n'
        '    for(var n = 0; n < mRoleButtons.length; n++)\n'
        '    {\n'
        '       oldButton = mRoleButtons[n];\n'
        '       oldButton.removeMovieClip();\n'
        '    }\n'
        '    mcList.clearList();',
    )
    text = once(
        text,
        '    var _loc20_ = 620;\n'
        '    var _loc8_ = 340;\n'
        '    var _loc18_ = 15;\n'
        '    if(mIsObserverModeEnabled == true)\n'
        '    {\n'
        '       _loc8_ = 280;\n'
        '    }',
        '    var _loc20_ = 620;\n'
        '    var _loc8_ = mLegacyRoleStep == 0 ? 540 : 390;\n'
        '    var _loc18_ = 15;\n'
        '    if(mIsObserverModeEnabled == true && mLegacyRoleStep == 0)\n'
        '    {\n'
        '       _loc8_ = 390;\n'
        '    }',
    )
    text = once(
        text,
        '       _loc2_.className = _loc4_.roleClass;',
        '       _loc2_.className = _loc4_.roleClass;\n'
        '       _loc2_.legacyHunterCard = _loc4_.legacyHunterCard == true;',
    )
    text = once(
        text,
        '       mRoleButtonClassMap[_loc2_.className] = _loc2_;\n'
        '       mRoleButtonClassNameMap[_loc2_.className] = _loc4_.displayName;',
        '       if(!_loc2_.legacyHunterCard)\n'
        '       {\n'
        '          mRoleButtonClassMap[_loc2_.className] = _loc2_;\n'
        '          mRoleButtonClassNameMap[_loc2_.className] = _loc4_.displayName;\n'
        '          var state = mLegacyRoleStates[_loc2_.className];\n'
        '          if(state != undefined)\n'
        '          {\n'
        '             updateClassEnableState(_loc2_.className,state.disabled,state.claimed,state.reason);\n'
        '          }\n'
        '       }',
    )
    text = once(
        text,
        '    mcList.initList();\n'
        '    if(_loc19_[mLastSelectedClass] != undefined)\n'
        '    {\n'
        '       mcList.selectIndex(_loc19_[mLastSelectedClass]);\n'
        '    }',
        '    mcListMount._x = (scripts.common.Style.RESOLUTION_DEFAULT_WIDTH - '
        'mRoleButtonData.length * (_loc8_ + _loc18_) + _loc18_) / 2;\n'
        '    mcList.initList();\n'
        '    if(mLegacyRoleStep == 0 && mLastSelectedClass != scripts.common.Team.CLASS_MONSTER)\n'
        '    {\n'
        '       mcList.selectIndex(0);\n'
        '    }\n'
        '    else if(_loc19_[mLastSelectedClass] != undefined)\n'
        '    {\n'
        '       mcList.selectIndex(_loc19_[mLastSelectedClass]);\n'
        '    }',
    )
    text = once(
        text,
        '       setRankedPlayScreenTitle("@ui_rp_choose_role","");',
        '       setRankedPlayScreenTitle("SELECT YOUR ROLE","");',
    )
    text = once(
        text,
        'function updateClassEnableState(sClass, bDisabled, sClaimedByUsername, sDisabledReason)\n'
        '{\n'
        '    var _loc1_ = mRoleButtonClassMap[sClass];',
        'function updateClassEnableState(sClass, bDisabled, sClaimedByUsername, sDisabledReason)\n'
        '{\n'
        '    mLegacyRoleStates[sClass] = {disabled:bDisabled,claimed:sClaimedByUsername,reason:sDisabledReason};\n'
        '    var _loc1_ = mRoleButtonClassMap[sClass];',
    )
    text = once(
        text,
        'function doRoleSelection()\n'
        '{\n'
        '    var _loc2_ = mcList.getSelectedIndex();\n'
        '    var _loc1_ = mcList.getItem(_loc2_);\n'
        '    if(isLocked(_loc1_) != true)',
        'function doRoleSelection()\n'
        '{\n'
        '    var _loc2_ = mcList.getSelectedIndex();\n'
        '    var _loc1_ = mcList.getItem(_loc2_);\n'
        '    if(mLegacyRoleStep == 0 && _loc1_.legacyHunterCard)\n'
        '    {\n'
        '       mLegacyRoleStep = 1;\n'
        '       setRankedPlayRoleData();\n'
        '       return;\n'
        '    }\n'
        '    if(isLocked(_loc1_) != true)',
    )
    text = once(
        text,
        'function onBack()\n'
        '{\n'
        '    flash.external.ExternalInterface.call("Back");',
        'function onBack()\n'
        '{\n'
        '    if(mLegacyRoleStep == 1)\n'
        '    {\n'
        '       mLegacyRoleStep = 0;\n'
        '       setRankedPlayRoleData();\n'
        '       return;\n'
        '    }\n'
        '    flash.external.ExternalInterface.call("Back");',
    )
    text = once(
        text,
        'var mLastSelectedClass = "";',
        'var mLastSelectedClass = "";\n'
        'var mLegacyRoleStep = 0;\n'
        'var mLegacyRoleStates = new Object();',
    )
    for contract in (
        'function openScreen(', 'function showComm(', 'function updateClassEnableState(',
        'function onLobbySizeChanged(', 'function cry_onController(',
        'flash.external.ExternalInterface.call("SelectClass",_loc1_.className);',
    ):
        if contract not in text:
            raise ValueError(f"Lost Stage 2 host callback: {contract}")
    return text


def run(*args: str) -> None:
    result = subprocess.run(args, capture_output=True, text=True, errors="replace")
    if result.returncode:
        raise RuntimeError(f"FFDec failed ({result.returncode}): {result.stdout[-2000:]} {result.stderr[-2000:]}")


def movie_tags(path: Path) -> tuple[bytes, list[tuple[int, bytes]]]:
    """Read bounded GFX framing to ensure only the one root script changed."""
    raw = path.read_bytes()
    if len(raw) < 16 or len(raw) > 32 * 1024 * 1024 or raw[:3] not in (b"GFX", b"CFX"):
        raise ValueError("Expected a bounded Stage 2 GFX/CFX movie")
    logical_size = int.from_bytes(raw[4:8], "little")
    if not 16 <= logical_size <= 32 * 1024 * 1024:
        raise ValueError("Invalid movie length")
    if raw[:3] == b"CFX":
        decoder = zlib.decompressobj()
        body = decoder.decompress(raw[8:], logical_size - 7)
        if not decoder.eof or decoder.unused_data or len(body) != logical_size - 8:
            raise ValueError("Invalid compressed movie framing")
    else:
        if len(raw) != logical_size:
            raise ValueError("Invalid uncompressed movie framing")
        body = raw[8:]
    rect_bits = body[0] >> 3
    if not 1 <= rect_bits <= 31:
        raise ValueError("Invalid stage rectangle")
    start = (5 + 4 * rect_bits + 7) // 8 + 4
    header = raw[:4] + body[:start]
    tags: list[tuple[int, bytes]] = []
    offset = start
    while offset + 2 <= len(body):
        word = int.from_bytes(body[offset:offset + 2], "little")
        offset += 2
        code, length = word >> 6, word & 63
        if length == 63:
            if offset + 4 > len(body):
                raise ValueError("Truncated long movie tag")
            length = int.from_bytes(body[offset:offset + 4], "little")
            offset += 4
        if length > len(body) - offset or len(tags) > 80000:
            raise ValueError("Malformed movie tag")
        tags.append((code, body[offset:offset + length]))
        offset += length
        if code == 0:
            break
    if not tags or tags[-1][0] != 0 or offset != len(body):
        raise ValueError("Movie must end at its End tag")
    return header, tags


def verify_only_role_script_changed(stock: Path, candidate: Path) -> None:
    original_header, original_tags = movie_tags(stock)
    candidate_header, candidate_tags = movie_tags(candidate)
    if original_header != candidate_header or len(original_tags) != len(candidate_tags):
        raise ValueError("Stage header or movie tag count changed")
    differences = [i for i, (old, new) in enumerate(zip(original_tags, candidate_tags)) if old != new]
    if len(differences) != 1 or original_tags[differences[0]][0] != 12 or candidate_tags[differences[0]][0] != 12:
        raise ValueError(f"Expected only the root DoAction tag to change; found {differences}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stock", type=Path, required=True, help="User-owned stock Stage 2 roleSelect.gfx")
    parser.add_argument("--ffdec", type=Path, required=True, help="FFDec CLI executable")
    parser.add_argument("--output", type=Path, required=True, help="NEW output roleSelect.gfx (outside game and stage)")
    args = parser.parse_args()
    stock = args.stock.resolve(strict=True)
    ffdec = args.ffdec.resolve(strict=True)
    output = args.output.absolute()
    scratch_root = Path(tempfile.gettempdir()).resolve(strict=True)
    if (stock.name.lower() != "roleselect.gfx" or output.name.lower() != "roleselect.gfx"
            or not output.parent.is_dir() or not output.parent.resolve().is_relative_to(scratch_root)
            or output.exists() or stock == output):
        parser.error("Use a new roleSelect.gfx destination under the system temp directory; never point at a game or stage")
    with tempfile.TemporaryDirectory(prefix="role-select-prototype-", dir=output.parent) as temp:
        exported = Path(temp) / "export"
        candidate = Path(temp) / "roleSelect.gfx"
        run(str(ffdec), "-onerror", "abort", "-export", "script", str(exported), str(stock))
        scripts = list(exported.rglob("frame_1/DoAction.as"))
        scripts = [p for p in scripts if p.parent.parent.name == "scripts"]
        if len(scripts) != 1:
            raise ValueError(f"Expected one stock root DoAction script, found {len(scripts)}")
        original = scripts[0].read_text(encoding="utf-8-sig")
        changed = patch_script(original)
        # Import ONLY the changed root script. Recompiling all decompiled
        # dependency scripts would silently modify unrelated Stage 2 behavior.
        import_dir = Path(temp) / "import"
        import_script = import_dir / "scripts" / "frame_1" / "DoAction.as"
        import_script.parent.mkdir(parents=True)
        import_script.write_text(changed, encoding="utf-8", newline="\n")
        run(str(ffdec), "-onerror", "abort", "-importScript", str(stock), str(candidate), str(import_dir))
        if not candidate.is_file():
            raise ValueError("FFDec did not publish the output movie")
        check_dir = Path(temp) / "check"
        run(str(ffdec), "-onerror", "abort", "-export", "script", str(check_dir), str(candidate))
        roots = [p for p in check_dir.rglob("frame_1/DoAction.as") if p.parent.parent.name == "scripts"]
        if len(roots) != 1:
            raise ValueError("Cannot re-export candidate root script")
        verified = roots[0].read_text(encoding="utf-8-sig")
        for required in ('SELECT YOUR ROLE', 'SELECT YOUR CLASS', 'mLegacyRoleStep',
                         'ExternalInterface.call("SelectClass"', 'updateClassEnableState'):
            if required not in verified:
                raise ValueError(f"Candidate lost required role behavior: {required}")
        verify_only_role_script_changed(stock, candidate)
        # Same-volume hard link publishes the checked bytes without overwriting
        # an existing file. Temporary work is removed after publication.
        os.link(candidate, output)
    print(json.dumps({"output": str(output), "sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
                      "source_sha256": hashlib.sha256(stock.read_bytes()).hexdigest(),
                      "status": "interaction prototype; not a Legacy visual port or in-game verified"}, indent=2))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError) as exc:
        print(f"Role prototype not built: {exc}", file=sys.stderr)
        sys.exit(1)
