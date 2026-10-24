"""Journal-independent game-file recovery and Restore-button regressions."""
from __future__ import annotations

import json
import os
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import orphaned_swap_recovery as recovery


class RecoverLostJournalTests(unittest.TestCase):
    def setUp(self):
        self.temp = TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        base = Path(self.temp.name)
        self.game = base / 'EvolveGame'
        self.game.mkdir()
        exe = self.game / 'bin64_SteamRetail' / 'Evolve.exe'
        exe.parent.mkdir()
        exe.write_bytes(b'exe')
        self.manifests = base / 'recovery-logs'

    def pair(self, relative, original=b'original', live=b'modded'):
        target = self.game / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(live)
        backup = Path(str(target) + recovery.SUFFIX)
        backup.write_bytes(original)
        return target, backup

    def test_journal_missing_preview_and_restore_preserves_all_mods(self):
        a, b = self.pair('Game/libs.pak')
        c, d = self.pair('Game/scripts.pak')
        inject, backup = self.pair('bin64_SteamRetail/inject.dll',
                                   b'i' * 4096, b'm' * 4096)
        before = recovery.plan_recovery(self.game)
        self.assertEqual(len(before), 3)
        self.assertEqual(len(list(self.game.rglob('*.customkey-recovery-mod-*'))), 0)
        with patch.object(recovery, 'ensure_game_closed'):
            manifest = recovery.recover(self.game, self.manifests)
        self.assertEqual(a.read_bytes(), b'original')
        self.assertEqual(c.read_bytes(), b'original')
        self.assertEqual(inject.read_bytes(), b'i' * 4096)
        self.assertFalse(any(self.game.rglob('*' + recovery.SUFFIX)))
        self.assertEqual(len(list(self.game.rglob('*.customkey-recovery-mod-*'))), 3)
        kept = sorted(p.read_bytes() for p in self.game.rglob('*.customkey-recovery-mod-*'))
        self.assertEqual(kept, sorted([b'modded', b'modded', b'm' * 4096]))
        record = json.loads(manifest.read_text(encoding='utf-8'))
        self.assertEqual(record['status'], 'complete')
        self.assertEqual(len(record['files']), 3)
        self.assertIsNone(recovery.recover(self.game, self.manifests))

    def test_backup_without_live_target_is_recovered(self):
        target, backup = self.pair('Game/one.pak')
        target.unlink()
        with patch.object(recovery, 'ensure_game_closed'):
            manifest = recovery.recover(self.game, self.manifests)
        self.assertEqual(target.read_bytes(), b'original')
        self.assertFalse(backup.exists())
        self.assertEqual(json.loads(manifest.read_text())['files'][0]['modified_copy'], None)

    def test_protect_existing_prepared_and_unsupported_backup(self):
        target, backup = self.pair('Game/a.pak')
        Path(str(target) + '.customkey-ready').write_bytes(b'prepared')
        with self.assertRaisesRegex(ValueError, 'prepared'):
            recovery.plan_recovery(self.game)
        self.assertEqual(target.read_bytes(), b'modded')
        self.assertEqual(backup.read_bytes(), b'original')
        Path(str(target) + '.customkey-ready').unlink()
        self.pair('Game/settings.ini')
        with self.assertRaisesRegex(ValueError, 'Unexpected backup type'):
            recovery.plan_recovery(self.game)

    def test_failure_rolling_back_one_file_does_not_lose_backup(self):
        target, backup = self.pair('Game/one.pak')
        real_rename = recovery.os.rename
        def fail_backup(source, destination):
            if Path(source) == backup:
                raise PermissionError('simulated locked backup')
            return real_rename(source, destination)
        with patch.object(recovery, 'ensure_game_closed'), \
             patch.object(recovery.os, 'rename', side_effect=fail_backup):
            with self.assertRaisesRegex(RuntimeError, 'Recovery stopped'):
                recovery.recover(self.game, self.manifests)
        self.assertEqual(target.read_bytes(), b'modded')
        self.assertEqual(backup.read_bytes(), b'original')
        logs = list(self.manifests.glob('SwapRecovery-*.json'))
        self.assertEqual(len(logs), 1)
        self.assertEqual(json.loads(logs[0].read_text())['status'],
                         'stopped_for_manual_review')

    def test_does_not_allow_manifest_directory_inside_game(self):
        self.pair('Game/one.pak')
        with patch.object(recovery, 'ensure_game_closed'):
            with self.assertRaisesRegex(ValueError, 'outside EvolveGame'):
                recovery.recover(self.game, self.game / 'Data')



if __name__ == '__main__':
    unittest.main()
