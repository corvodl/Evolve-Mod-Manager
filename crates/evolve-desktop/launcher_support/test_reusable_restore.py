"""Regression tests for verified recovery of clean-phase stranded original files."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import controlled_swap as swap
import reusable_restore


class StrandedBackupTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        base = Path(self.temp.name)
        self.game = base / 'game'
        self.stage = base / 'Setup_sample' / 'staged'
        self.game.joinpath('Game').mkdir(parents=True)
        self.game.joinpath('bin64_SteamRetail').mkdir(parents=True)
        self.stage.joinpath('paks', 'Game').mkdir(parents=True)
        (self.stage.parent / 'RSAKeyData.bin').write_bytes(bytes([0x23]) * 140)

        self.original_pak = b'original PAK bytes'
        self.custom_pak = b'custom signed PAK bytes'
        self.original_hook = bytearray([0x55] * 4096)
        self.original_hook[0x870:0x870 + 140] = bytes([0x23] * 140)
        self.custom_hook = bytearray(self.original_hook)
        self.custom_hook[0x870:0x870 + 140] = bytes([0x31] * 140)

        pak = self.game / 'Game' / 'UI_Assets.pak'
        hook = self.game / 'bin64_SteamRetail' / 'inject.dll'
        staged_pak = self.stage / 'paks' / 'Game' / 'UI_Assets.pak'
        staged_hook = self.stage / 'inject-custom.dll'
        pak.write_bytes(self.custom_pak)
        hook.write_bytes(self.custom_hook)
        staged_pak.write_bytes(self.custom_pak)
        staged_hook.write_bytes(self.custom_hook)
        self.pak_backup = Path(str(pak) + swap.BACKUP)
        self.hook_backup = Path(str(hook) + swap.BACKUP)
        self.pak_backup.write_bytes(self.original_pak)
        self.hook_backup.write_bytes(self.original_hook)

        (self.stage / 'rekey_plan.json').write_text(json.dumps({
            'source_root': str(self.game),
            'entries': [{
                'path': 'Game/UI_Assets.pak',
                'kind': 'signed-encrypted',
                'sha256': self._sha(self.original_pak)
            }]
        }), encoding='utf-8')
        self.state = {
            'version': 1, 'phase': 'prepared',
            'game': str(self.game), 'stage': str(self.stage),
            'count': 2,
            'files': [
                self.row('Game/UI_Assets.pak', staged_pak, pak, self.custom_pak),
                self.row('bin64_SteamRetail/inject.dll', staged_hook, hook, self.custom_hook),
            ],
        }
        self.journal = base / 'controlled_swap_state.json'
        self.journal.write_text(json.dumps(self.state), encoding='utf-8')
        self.addCleanup(patch.stopall)
        patch.object(swap, 'STATE_FILE', self.journal).start()

    @staticmethod
    def _sha(data):
        import hashlib
        return hashlib.sha256(data).hexdigest()

    def row(self, name, source, target, custom):
        return {
            'name': name, 'source': str(source), 'target': str(target),
            'ready': str(target) + swap.READY,
            'backup': str(target) + swap.BACKUP,
            'sha256': self._sha(custom), 'bytes': len(custom),
        }

    def run_restore(self):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return reusable_restore.main()

    def test_verified_prepared_phase_recovers_stranded_originals(self):
        self.assertEqual(self.run_restore(), 0)
        pak = self.game / 'Game' / 'UI_Assets.pak'
        hook = self.game / 'bin64_SteamRetail' / 'inject.dll'
        self.assertEqual(pak.read_bytes(), self.original_pak)
        self.assertEqual(hook.read_bytes(), self.original_hook)
        self.assertFalse(self.pak_backup.exists())
        self.assertFalse(self.hook_backup.exists())
        self.assertEqual(Path(str(pak) + swap.READY).read_bytes(), self.custom_pak)
        self.assertEqual(swap.require_state()['phase'], 'prepared')

    def test_restored_phase_with_backup_is_not_mistaken_for_clean(self):
        self.state['phase'] = 'restored'
        self.journal.write_text(json.dumps(self.state), encoding='utf-8')
        self.assertEqual(self.run_restore(), 0)
        self.assertFalse(self.hook_backup.exists())

    def test_tampered_original_backup_is_preserved_without_mutation(self):
        self.pak_backup.write_bytes(b'foreign bytes')
        self.assertEqual(self.run_restore(), 1)
        self.assertEqual(self.pak_backup.read_bytes(), b'foreign bytes')
        self.assertTrue(self.hook_backup.exists())
        self.assertEqual(swap.require_state()['phase'], 'prepared')

    def test_wrong_original_hook_key_is_preserved(self):
        self.hook_backup.write_bytes(bytes([0x46] * 4096))
        self.assertEqual(self.run_restore(), 1)
        self.assertTrue(self.hook_backup.exists())
        self.assertTrue(self.pak_backup.exists())

    def test_duplicate_original_target_requires_manual_inspection(self):
        (self.game / 'Game' / 'UI_Assets.pak').write_bytes(self.original_pak)
        self.assertEqual(self.run_restore(), 1)
        self.assertTrue(self.pak_backup.exists())
        self.assertEqual(swap.require_state()['phase'], 'prepared')

    def test_forged_backup_path_is_refused(self):
        self.state['files'][0]['backup'] = str(self.journal)
        self.journal.write_text(json.dumps(self.state), encoding='utf-8')
        self.assertEqual(self.run_restore(), 1)
        self.assertTrue(self.pak_backup.exists())


if __name__ == '__main__':
    unittest.main()
