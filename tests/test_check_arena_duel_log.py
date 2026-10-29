"""Strict 2-player Monster Arena log-checker regression tests."""
import importlib.util
from pathlib import Path
import unittest

MOD = Path(__file__).resolve().parents[1] / 'scripts' / 'experiments' / 'check_arena_duel_log.py'
spec = importlib.util.spec_from_file_location('check_arena_duel_log', MOD)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

START = '''<04:00:01> <GameLobby> StartGame gathering initial users
<04:00:01> <GameLobby> userId:a name:Alpha team:3, isHost:1
<04:00:01> <GameLobby> userId:b name:Beta team:0, isHost:0
'''
SLOTS = '''<04:00:03> [PlayerSlot_monster_00]
<04:00:03>   team: monster
<04:00:03>   class: MONSTER
<04:00:03>   character: GoliathA - Goliath
<04:00:03>   stage: 2
<04:00:03>   perks:  none
<04:00:03>   inWorld: <null>
<04:00:03>   user: user_Alpha (userSlot: PlayerSlot_monster_00)
<04:00:03> [PlayerSlot_merc_00]
<04:00:03>   team: merc
<04:00:03>   class: MONSTER
<04:00:03>   character: KrakenA - Kraken
<04:00:03>   stage: 2
<04:00:03>   inWorld: <null>
<04:00:03>   user: user_Beta (userSlot: PlayerSlot_merc_00)
'''


class CheckArenaDuelLogTests(unittest.TestCase):
    def test_valid_two_monster_roster(self):
        failures, notes = mod.assess(START + SLOTS)
        self.assertEqual(failures, [])
        self.assertTrue(notes)

    def test_rejects_third_human(self):
        extra = '<04:00:01> <GameLobby> userId:c name:Gamma team:0, isHost:0\n'
        failures, _ = mod.assess(START + extra + SLOTS)
        self.assertTrue(any('exactly 2' in x for x in failures))

    def test_rejects_hunter_in_reserved_slot(self):
        occupied = '''<04:00:03> [PlayerSlot_merc_01]
<04:00:03>   team: merc
<04:00:03>   class: ASSAULT
<04:00:03>   character: AssaultE - Markov
<04:00:03>   inWorld: <null>
<04:00:03>   user: <null>
'''
        failures, _ = mod.assess(START + SLOTS + occupied)
        self.assertTrue(any('hunter role/character' in x for x in failures))

    def test_rejects_one_human(self):
        one = ''.join(START.splitlines(keepends=True)[:2])
        failures, _ = mod.assess(one + SLOTS)
        self.assertTrue(failures)

    def test_no_prior_match_leakage(self):
        one = '<04:01:00> <GameLobby> StartGame gathering initial users\n<04:01:00> <GameLobby> userId:a name:Alpha team:3, isHost:1\n'
        failures, _ = mod.assess(START + SLOTS + one)
        self.assertTrue(any('exactly 2' in x for x in failures))


if __name__ == '__main__':
    unittest.main()
