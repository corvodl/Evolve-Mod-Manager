"""Regression tests for stale prepared PAK reuse across enabled mod profiles.

The stock installed game, original backup suffix, and recovery journal must
never be silently replaced just because the staged mod build changed.
"""
import hashlib
import json
from pathlib import Path
from types import SimpleNamespace
from unittest import mock
import tempfile
import unittest

import controlled_swap as swap


def digest(data):
    return hashlib.sha256(data).hexdigest()


class PreparedProfileCacheTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.game = root / "game"
        self.stage = root / "stage"
        self.journal_dir = root / "swap"
        self.game.mkdir()
        self.stage.mkdir()
        self.journal_dir.mkdir()
        self.rel = "Game/UI_Assets.pak"
        self.target = self.game / self.rel
        self.source = self.stage / "paks" / self.rel
        self.target.parent.mkdir(parents=True)
        self.source.parent.mkdir(parents=True)
        self.original = b"original stock signed pak fixture"
        self.modified = b"newly modified profile pak fixture"
        self.previous = b"previous profile prepared pak fixture"
        self.target.write_bytes(self.original)
        self.source.write_bytes(self.modified)
        self.ready = Path(str(self.target) + swap.READY)
        self.backup = Path(str(self.target) + swap.BACKUP)

        original_key = bytes(i % 256 for i in range(140))
        custom_key = bytes((i + 1) % 256 for i in range(140))
        original_shim = bytearray(4096)
        original_shim[0x870:0x870 + 140] = original_key
        custom_shim = bytearray(4096)
        custom_shim[0x870:0x870 + 140] = custom_key
        (root / "RSAKeyData.bin").write_bytes(original_key)
        (self.stage / "mykeys").mkdir()
        (self.stage / "mykeys" / "public_key.bin").write_bytes(custom_key)
        (self.game / "bin64_SteamRetail").mkdir()
        (self.game / "bin64_SteamRetail" / "inject.dll").write_bytes(original_shim)
        (self.stage / "inject-custom.dll").write_bytes(custom_shim)
        self.custom_shim = bytes(custom_shim)
        (self.stage / "rekey_plan.json").write_text(json.dumps({
            "source_root": str(self.game),
            "blocked": [],
            "entries": [
                {
                    "path": self.rel,
                    "kind": "signed-encrypted",
                    "sha256": digest(self.original),
                    "bytes": len(self.original),
                }
            ],
        }), encoding="utf-8")
        (self.stage / "stage_status.json").write_text(json.dumps({
            "status": "fully-staged-offline",
            "count": 1,
            "signer": "local-custom",
            "shim_sha256": digest(self.custom_shim),
        }), encoding="utf-8")
        (self.stage / "active_mod_paks.json").write_text(json.dumps({
            "schema": 1, "targets": [self.rel],
        }), encoding="utf-8")
        self.state = self.journal_dir / "controlled_swap_state.json"
        patch = mock.patch.object(swap, "STATE_FILE", self.state)
        patch.start()
        self.addCleanup(patch.stop)
        self.args = SimpleNamespace(
            game_root=str(self.game),
            stage_dir=str(self.stage),
        )

    def prepare(self):
        with mock.patch.object(swap, "all_closed"):
            swap.prepare(self.args)

    def stale(self):
        return list(self.ready.parent.glob(self.ready.name + ".stale-*"))

    def test_rebuild_preserves_old_ready_and_prepares_new_profile(self):
        self.ready.write_bytes(self.previous)
        self.state.write_text(json.dumps({
            "phase": "restored", "files": [], "count": 0,
        }), encoding="utf-8")
        self.prepare()
        self.assertEqual(self.target.read_bytes(), self.original)
        self.assertEqual(self.ready.read_bytes(), self.modified)
        self.assertEqual(len(self.stale()), 1)
        self.assertEqual(self.stale()[0].read_bytes(), self.previous)
        state = swap.read_json(self.state)
        self.assertEqual(state["phase"], "prepared")
        self.assertEqual(state["count"], 2)
        self.assertEqual(state["files"][0]["sha256"], digest(self.modified))
        self.assertFalse(self.backup.exists())

    def test_identical_prepared_file_reused_without_extra_copy(self):
        self.ready.write_bytes(self.modified)
        self.prepare()
        self.assertEqual(self.ready.read_bytes(), self.modified)
        self.assertEqual(self.stale(), [])
        self.assertEqual(swap.read_json(self.state)["phase"], "prepared")

    def test_active_journal_blocks_without_touching_prepared_copy(self):
        self.ready.write_bytes(self.previous)
        self.state.write_text(json.dumps({
            "phase": "swapped", "files": [], "count": 0,
        }), encoding="utf-8")
        with self.assertRaisesRegex(RuntimeError, "existing state journal"):
            self.prepare()
        self.assertEqual(self.ready.read_bytes(), self.previous)
        self.assertEqual(self.stale(), [])
        self.assertEqual(self.target.read_bytes(), self.original)

    def test_original_backup_blocks_cache_replacement(self):
        self.ready.write_bytes(self.previous)
        self.backup.write_bytes(b"original recovery backup")
        with self.assertRaisesRegex(RuntimeError, "Existing backup|Original backup"):
            self.prepare()
        self.assertEqual(self.ready.read_bytes(), self.previous)
        self.assertEqual(self.backup.read_bytes(), b"original recovery backup")
        self.assertEqual(self.stale(), [])

    def test_tampered_original_does_not_trigger_stale_quarantine(self):
        self.ready.write_bytes(self.previous)
        self.target.write_bytes(b"modified installed file")
        with self.assertRaisesRegex(RuntimeError, "Installed stock PAK differs"):
            self.prepare()
        self.assertEqual(self.ready.read_bytes(), self.previous)
        self.assertEqual(self.target.read_bytes(), b"modified installed file")
        self.assertEqual(self.stale(), [])

    def test_symlink_ready_is_not_trusted(self):
        self.ready.symlink_to(self.source)
        with self.assertRaisesRegex(RuntimeError, "Unsafe prepared copy"):
            self.prepare()
        self.assertTrue(self.ready.is_symlink())
        self.assertEqual(self.stale(), [])

    def test_abandoned_partial_is_preserved_independently(self):
        self.ready.write_bytes(self.previous)
        partial = Path(str(self.ready) + ".partial")
        partial.write_bytes(b"interrupted copy")
        self.prepare()
        self.assertEqual(self.target.read_bytes(), self.original)
        self.assertEqual(self.ready.read_bytes(), self.modified)
        self.assertEqual(len(self.stale()), 1)
        self.assertEqual(len(list(partial.parent.glob(partial.name + ".stale-*"))), 1)
        self.assertFalse(partial.exists())


if __name__ == "__main__":
    unittest.main()
