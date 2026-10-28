#!/usr/bin/env python3
"""Synthetic, no-game-file tests for the CharacterStage inventory helper."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


spec = importlib.util.spec_from_file_location(
    "stage_audit", Path(__file__).with_name("audit-character-stage-dependencies.py")
)
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class CharacterStageInventoryTests(unittest.TestCase):
    def test_rejects_unsafe_paths_and_does_not_guess_missing_assets(self):
        for path in ("../escape.pak", "/absolute.dds", "C:/game/file.cdf", "a//b.xml"):
            self.assertIsNone(audit.member(path))
        self.assertEqual(audit.member(r"Objects\Hunters\Parnell.cdf"),
                         "objects/hunters/parnell.cdf")

    def test_resources_and_xml_references_are_bounded_and_inventory_is_explicit(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            level = root / "level"
            data = root / "data"
            level.mkdir()
            data.mkdir()
            (level / "resourcelist.txt").write_text("objects/a.cgf\n../unsafe.dds\n", encoding="utf-8")
            (data / "entities.xml").write_text('<Entities Model="objects/b.cdf"/>', encoding="utf-8")
            report = root / "stock.json"
            report.write_text(json.dumps({"entries": [{"path": "Objects/B.cdf"}]}), encoding="utf-8")
            self.assertEqual(audit.references(level, data), {"objects/a.cgf", "objects/b.cdf"})
            self.assertEqual(audit.read_inventory(report), {"objects/b.cdf"})
            self.assertEqual(audit.extracted_paths(data), {"entities.xml"})
            (data / "entities.xml").write_text('<!DOCTYPE x [<!ENTITY leak SYSTEM "file:///x">]><x/>')
            with self.assertRaises(ValueError):
                audit.references(level, data)


if __name__ == "__main__":
    unittest.main()
