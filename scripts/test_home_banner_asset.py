#!/usr/bin/env python3
"""Verify that the original Home banner and both Windows package routes agree."""
from __future__ import annotations

import hashlib
from pathlib import Path
import struct
import unittest

ROOT = Path(__file__).resolve().parents[1]
BANNER = ROOT / "assets" / "default_loading.dds"
EXPECTED_SHA256 = "2f02f19b085dc7edc5f88b730121ca511bf499b2c3ef1045f4ebe020da3fff5a"


class HomeBannerAssetTests(unittest.TestCase):
    def test_original_bc1_dds_is_intact(self):
        data = BANNER.read_bytes()
        self.assertEqual(hashlib.sha256(data).hexdigest(), EXPECTED_SHA256)
        self.assertEqual(data[:4], b"DDS ")
        self.assertEqual(struct.unpack_from("<I", data, 4)[0], 124)
        self.assertEqual(struct.unpack_from("<I", data, 12)[0], 1080)
        self.assertEqual(struct.unpack_from("<I", data, 16)[0], 1920)
        self.assertEqual(data[84:88], b"DXT1")
        self.assertEqual(len(data), 128 + ((1920 + 3) // 4) * ((1080 + 3) // 4) * 8)

    def test_home_widget_loads_installed_assets_folder(self):
        code = (ROOT / "crates/evolve-desktop/src/private_pages.rs").read_text()
        self.assertIn('base.join("assets").join("default_loading.dds")', code)
        self.assertIn("decode_home_dds(&data)", code)
        self.assertIn("from_rgba(width, height, rgba)", code)

    def test_bundled_fallback_survives_executable_only_updates(self):
        code = (ROOT / "crates/evolve-desktop/src/private_pages.rs").read_text()
        self.assertIn('include_bytes!("../../../assets/default_loading.dds")', code)
        self.assertIn("decode_home_dds(bundled)", code)
        self.assertIn("from_rgba(", code)

    def test_portable_nightly_includes_banner(self):
        source = (ROOT / ".github/workflows/nightly.yml").read_text()
        self.assertIn('"assets/default_loading.dds"', source)
        self.assertIn('Join-Path $assetDir "default_loading.dds"', source)
        self.assertIn('Get-FileHash -LiteralPath $banner -Algorithm SHA256', source)

    def test_windows_installer_includes_banner(self):
        source = (ROOT / "installer/EvolveModManager.iss").read_text()
        self.assertIn('Source: "..\\assets\\default_loading.dds"', source)
        self.assertIn('DestDir: "{app}\\assets"', source)


if __name__ == "__main__":
    unittest.main()
