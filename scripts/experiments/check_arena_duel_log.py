#!/usr/bin/env python3
"""Conservative log-only gate for a future Stage 2 two-human Monster Arena.

A passing log shows two initialized, uniquely owned monster-role slots and no
active hunter-role slots. It cannot prove that a third connection is rejected,
that AI actors are absent, or that input/damage work: those need live tests.
"""
from __future__ import annotations

import argparse
import re
from pathlib import Path

SLOT_RE = re.compile(r'^\[PlayerSlot_(monster_\d\d|merc_\d\d)\]$')
ATTR_RE = re.compile(r'^\s+(team|class|character|inWorld|user):\s*(.*?)\s*$')
OTHER_FIELD_RE = re.compile(r'^\s+[A-Za-z]+:\s*.*$')
USER_RE = re.compile(r'userId:\S+\s+name:(.*?)\s+team:(\d+)\b')
MISSING = {'', '<null>', 'none', 'disabled', 'unknown'}


def assess(log: str) -> tuple[list[str], list[str]]:
    failures: list[str] = []
    notes: list[str] = []
    lines = []
    for raw in log.splitlines():
        if raw.startswith('<') and '> ' in raw:
            raw = raw.split('> ', 1)[1]
        lines.append(raw.rstrip())

    # Only inspect the final start-game cycle, avoiding success from prior runs.
    starts = [i for i, line in enumerate(lines) if '<GameLobby> StartGame gathering initial users' in line]
    if not starts:
        return ['No StartGame initial-users record in log'], notes
    lines = lines[starts[-1]:]

    users: list[tuple[str, str]] = []
    for line in lines[1:]:
        if '<GameLobby> userId:' in line:
            match = USER_RE.search(line)
            if match:
                users.append((match.group(1).strip(), match.group(2)))
        elif users:
            break
    names = {name for name, _ in users}
    if len(users) != 2 or len(names) != 2:
        failures.append(f'Expected exactly 2 distinct human lobby users, found {len(users)} records / {len(names)} names')

    # Inspect the slot-state block for the FINAL game cycle only.
    roster: dict[str, dict[str, str]] = {}
    current: str | None = None
    for line in lines:
        match = SLOT_RE.match(line)
        if match:
            current = match.group(1)
            roster[current] = {}
            continue
        if current is not None:
            value = ATTR_RE.match(line)
            if value:
                roster[current][value.group(1)] = value.group(2)
            elif not OTHER_FIELD_RE.match(line):
                current = None

    for slot, team in [('monster_00', 'monster'), ('merc_00', 'merc')]:
        entry = roster.get(slot)
        if not entry:
            failures.append(f'PlayerSlot_{slot}: missing final roster entry')
            continue
        if entry.get('team', '').lower() != team:
            failures.append(f'PlayerSlot_{slot}: team must be {team!r}')
        if entry.get('class', '').upper() != 'MONSTER':
            failures.append(f'PlayerSlot_{slot}: must have MONSTER actor class')
        if 'user_' not in entry.get('user', ''):
            failures.append(f'PlayerSlot_{slot}: no assigned human owner')

    owned = []
    for slot in ['monster_00', 'merc_00']:
        entry = roster.get(slot, {})
        match = re.search(r'\buser_([^\s(]+)', entry.get('user', ''))
        if match:
            owned.append(match.group(1))
    if len(owned) == 2 and (len(set(owned)) != 2 or set(owned) != names):
        failures.append('The two slots must belong to the two distinct initial human users')

    for slot in ['merc_01', 'merc_02', 'merc_03']:
        entry = roster.get(slot)
        if not entry:
            notes.append(f'PlayerSlot_{slot}: not logged; absence is not proof of disabled state')
            continue
        for key in ['user', 'inWorld']:
            if entry.get(key, '').strip().lower() not in MISSING:
                failures.append(f'PlayerSlot_{slot}: {key} is active')
        if entry.get('class', '').strip().lower() not in MISSING or entry.get('character', '').strip().lower() not in MISSING:
            failures.append(f'PlayerSlot_{slot}: hunter role/character still assigned (not disabled)')

    if 'monster_01' in roster:
        failures.append('PlayerSlot_monster_01 appeared; test must retain native monster count 1')
    notes.append('Log-only: cannot prove 2-player matchmaking CAP, third-client rejection, no bot actors, or damage.')
    notes.append('Verify those conditions separately in a live two-client Arena test.')
    return failures, notes


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('game_log', type=Path)
    args = parser.parse_args()
    failures, notes = assess(args.game_log.read_text(encoding='utf-8', errors='replace'))
    for value in failures:
        print('FAIL:', value)
    for value in notes:
        print('NOTE:', value)
    if failures:
        return 1
    print('PRELIMINARY LOG PASS — live two-client/third-client rejection tests remain required')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
