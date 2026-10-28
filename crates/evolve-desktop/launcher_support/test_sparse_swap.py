"""Selective PAK swapping must never include unrelated signed stock archives."""
import json
from pathlib import Path
import tempfile
import unittest

import controlled_swap as swap


class SparseSwapPlanTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.game = root / 'game'
        self.stage = root / 'stage'
        self.game.mkdir()
        self.stage.mkdir()
        self.names = ['Game/first.pak', 'Game/second.pak', 'Game/third.pak']
        (self.stage / 'rekey_plan.json').write_text(json.dumps({
            'source_root': str(self.game),
            'entries': [{'kind': 'signed-encrypted', 'path': name}
                        for name in self.names],
        }))
        (self.stage / 'stage_status.json').write_text(json.dumps({
            'status': 'fully-staged-offline', 'count': len(self.names)
        }))

    def select(self, targets):
        (self.stage / 'active_mod_paks.json').write_text(
            json.dumps({'schema': 1, 'targets': targets}))
        return swap.list_files(self.game, self.stage)[0]

    def test_only_enabled_pak_and_required_runtime_hook(self):
        files = self.select(['Game/second.pak'])
        self.assertEqual([item['name'].replace('\\', '/') for item in files],
                         ['Game/second.pak', 'bin64_SteamRetail/inject.dll'])

    def test_two_mods_do_not_include_unmodified_third(self):
        files = self.select(['Game/third.pak', 'Game/first.pak'])
        self.assertEqual([item['name'].replace('\\', '/') for item in files],
                         ['Game/first.pak', 'Game/third.pak',
                          'bin64_SteamRetail/inject.dll'])

    def test_legacy_missing_manifest_retains_full_inventory(self):
        self.assertEqual(len(swap.list_files(self.game, self.stage)[0]), 4)

    def test_invalid_selection_fails_not_full_swap(self):
        for paths in ([], ['Game/missing.pak'],
                      ['Game/first.pak', 'Game/first.pak'],
                      ['Game/../third.pak'], ['/etc/pak.pak'],
                      ['Game' + chr(92) + 'first.pak']):
            with self.subTest(paths=paths), self.assertRaises(RuntimeError):
                self.select(paths)

    def test_bad_schema_cannot_reenable_full_swap(self):
        (self.stage / 'active_mod_paks.json').write_text('{}')
        with self.assertRaises(RuntimeError):
            swap.list_files(self.game, self.stage)


if __name__ == '__main__':
    unittest.main()
