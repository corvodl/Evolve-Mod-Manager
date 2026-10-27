#!/usr/bin/env python3
"""Synthetic safety checks; never use an installed game or manager stage."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from types import SimpleNamespace


spec = importlib.util.spec_from_file_location(
    "prepared_refresh", Path(__file__).with_name("refresh-prepared-selection.py")
)
refresh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(refresh)


class PreparedRefreshTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.game = root / "game"
        self.stage = root / "stage"
        self.swap = root / "swap"
        for folder in (self.game, self.stage, self.swap):
            folder.mkdir()
        (self.game / "Game").mkdir()
        (self.stage / "paks" / "Game").mkdir(parents=True)
        (self.game / "bin64_SteamRetail").mkdir()
        original = self.game / "Game" / "UI_Data.pak"
        source = self.stage / "paks" / "Game" / "UI_Data.pak"
        original.write_bytes(b"original")
        source.write_bytes(b"new signed candidate")
        ready = Path(str(original) + refresh.READY)
        ready.write_bytes(b"old signed candidate")
        hook = self.game / "bin64_SteamRetail" / "inject.dll"
        hook.write_bytes(b"original shim")
        custom = self.stage / "inject-custom.dll"
        custom.write_bytes(b"custom shim")
        Path(str(hook) + refresh.READY).write_bytes(b"custom shim")
        (self.stage / "stage_status.json").write_text(json.dumps({
            "status": "fully-staged-offline", "count": 1,
        }))
        (self.stage / "rekey_plan.json").write_text(json.dumps({
            "source_root": str(self.game), "blocked": False,
            "entries": [{"kind": "signed-encrypted", "path": "Game/UI_Data.pak",
                         "signed_name": "ui_data.pak"}],
        }))
        self.records = [
            {"name": "Game/UI_Data.pak", "target": str(original), "source": str(source),
             "ready": str(ready), "backup": str(original) + refresh.BACKUP,
             "sha256": refresh.digest(ready)},
            {"name": "bin64_SteamRetail/inject.dll", "target": str(hook),
             "source": str(custom), "ready": str(hook) + refresh.READY,
             "backup": str(hook) + refresh.BACKUP,
             "sha256": refresh.digest(custom)},
        ]
        self.journal = {"version": 1, "phase": "prepared", "count": 2,
                        "game": str(self.game), "stage": str(self.stage),
                        "files": self.records}
        self.save()

    def save(self):
        (self.swap / "controlled_swap_state.json").write_text(json.dumps(self.journal))

    def test_only_identifies_stale_ready_and_preserves_inputs(self):
        before = (self.swap / "controlled_swap_state.json").read_bytes()
        journal, changes = refresh.plan_refresh(self.stage, self.swap)
        self.assertEqual(journal, self.journal)
        self.assertEqual(len(changes), 1)
        self.assertEqual(changes[0][2].name, "UI_Data.pak")
        self.assertEqual((self.swap / "controlled_swap_state.json").read_bytes(), before)
        self.assertTrue(Path(self.records[0]["ready"]).is_file())

    def test_rejects_active_or_backup_or_retarged_journal(self):
        self.journal["phase"] = "swapped"
        self.save()
        with self.assertRaises(ValueError):
            refresh.plan_refresh(self.stage, self.swap)
        self.journal["phase"] = "prepared"
        self.save()
        Path(self.records[0]["backup"]).write_bytes(b"original backup")
        with self.assertRaises(ValueError):
            refresh.plan_refresh(self.stage, self.swap)
        Path(self.records[0]["backup"]).unlink()
        self.records[0]["target"] = str(self.game / "Game" / "other.pak")
        self.save()
        with self.assertRaises((ValueError, FileNotFoundError)):
            refresh.plan_refresh(self.stage, self.swap)

    def test_apply_preserves_old_journal_and_ready_without_touching_original(self):
        (self.stage / "mykeys").mkdir()
        (self.stage / "mykeys" / "public_key.bin").write_bytes(b"test key")
        cli = Path(self.temp.name) / "inspector.exe"
        cli.write_bytes(b"synthetic inspector")
        original = Path(self.records[0]["target"])
        old_original = original.read_bytes()
        old_journal = (self.swap / "controlled_swap_state.json").read_bytes()
        with patch.object(refresh, "no_games"), patch.object(
            refresh.subprocess, "run", return_value=SimpleNamespace(returncode=0)
        ):
            refresh.refresh(self.stage, self.swap, cli, True)
        self.assertEqual(original.read_bytes(), old_original)
        self.assertFalse(Path(self.records[0]["ready"]).exists())
        stale = Path(self.records[0]["ready"] + ".stale-" + self.records[0]["sha256"][:16])
        self.assertEqual(stale.read_bytes(), b"old signed candidate")
        copies = list(self.swap.glob("controlled_swap_state.json.pre-refresh-*"))
        self.assertEqual(len(copies), 1)
        self.assertEqual(copies[0].read_bytes(), old_journal)
        self.assertEqual(refresh.checked_json(self.swap / "controlled_swap_state.json")["phase"], "restored")

    def test_interrupted_rename_resumes_only_with_verified_stale_bytes(self):
        ready = Path(self.records[0]["ready"])
        stale = Path(str(ready) + ".stale-" + self.records[0]["sha256"][:16])
        ready.rename(stale)
        _, changes = refresh.plan_refresh(self.stage, self.swap)
        self.assertEqual(len(changes), 1)
        stale.write_bytes(b"tampered")
        with self.assertRaises(ValueError):
            refresh.plan_refresh(self.stage, self.swap)


if __name__ == "__main__":
    unittest.main()
