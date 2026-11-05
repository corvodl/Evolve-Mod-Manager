"""Opt-in source-provenance tests for original Evolve Tutorial .lyr CDFs.

No copyrighted game assets are committed. Point EVOLVE_TEST_TUTORIAL_ZIP to
the exact owner-provided Tutorial.zip on an authorized test machine.
"""
import os
from pathlib import Path
import re
import unittest
import zipfile

CDF = re.compile(rb"(?:characters|objects)/[a-z0-9_./-]+\.cdf", re.I)


class TutorialActualCDFSources(unittest.TestCase):
    def test_owner_original_tutorial_cdf_placements_opt_in(self):
        source = os.getenv("EVOLVE_TEST_TUTORIAL_ZIP")
        if not source:
            self.skipTest("Set EVOLVE_TEST_TUTORIAL_ZIP to an original authorized source archive")
        filename = Path(source)
        self.assertTrue(filename.is_file(), "Original Tutorial source ZIP is missing")
        refs = []
        layers = set()
        with zipfile.ZipFile(filename) as archive:
            names = archive.namelist()
            for member in names:
                if not member.lower().endswith(".lyr"):
                    continue
                self.assertFalse(member.startswith("/") or ".." in member.split("/"))
                # Original level files are bounded to protect the audit worker.
                info = archive.getinfo(member)
                self.assertLess(info.file_size, 48 * 1024 * 1024)
                raw = archive.read(member)
                matching = [
                    item.decode("ascii").lower()
                    for item in CDF.findall(raw)
                ]
                if matching:
                    layers.add(member)
                    refs.extend(matching)
        # Owner's original 2026 Tutorial source fixture:
        # 27 authored CDF references from 7 layers, naming 19 distinct CDFs.
        self.assertEqual(len(refs), 27)
        self.assertEqual(len(set(refs)), 19)
        self.assertEqual(len(layers), 7)
        self.assertIn("objects/dropship/dropship_laurieann.cdf", refs)
        self.assertIn("characters/monsters/goliath/goliath3.cdf", refs)
        # These are only authored references, not verified render-mesh counts.
        # Full CDF body/rig files reside in separate same-install PAKs.


if __name__ == "__main__":
    unittest.main()
