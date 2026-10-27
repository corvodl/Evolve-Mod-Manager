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

    def test_home_art_covers_viewport_without_a_separate_banner_row(self):
        code = (ROOT / "crates/evolve-desktop/src/private_pages.rs").read_text()
        home = code.split("pub(super) fn home(state: &State)", 1)[1].split(
            "fn docs_spec(", 1
        )[0]
        self.assertIn("stack![background, shade, content]", home)
        self.assertIn("image(handle)", home)
        self.assertIn(".content_fit(iced::ContentFit::Cover)", home)
        self.assertIn("scrollable(", home)
        self.assertNotIn("Length::Fixed(280.0)", home)
        self.assertNotIn("column![container(picture)", home)

    def test_home_keeps_background_but_brings_workspace_into_view(self):
        code = (ROOT / "crates/evolve-desktop/src/private_pages.rs").read_text()
        home = code.split("pub(super) fn home(state: &State)", 1)[1].split(
            "fn docs_spec(", 1
        )[0]
        self.assertIn("stack![background, shade, content]", home)
        self.assertIn("home_play_surface(quick, wallpaper)", home)
        self.assertIn("wallpaper.then(home_banner).flatten()", home)
        self.assertIn(".spacing(13)", home)
        self.assertIn("paths_selected", home)
        self.assertIn("Create your first mod", home)
        self.assertNotIn("Length::Fixed(280.0)", home)

    def test_home_news_is_thumbnail_and_art_is_brighter(self):
        code = (ROOT / "crates/evolve-desktop/src/private_pages.rs").read_text()
        news = code.split("fn latest_announcement(", 1)[1].split(
            "fn home_banner(", 1
        )[0]
        self.assertRegex(news, r"\.image_urls\s*\.iter\(\)")
        self.assertIn(".find(|url| crate::whats_new::allowed_image_url(url))", news)
        self.assertIn("row![details, preview]", news)
        self.assertIn("Length::Fixed(125.0)", news)
        self.assertIn("Length::Fixed(340.0)", news)
        self.assertIn('Page::WhatsNew', news)
        self.assertIn("as u16 * 59 / 100", code)
        self.assertIn("backdrop.a = if wallpaper { 0.88 } else { 1.0 }", code)
        self.assertIn("backdrop.a = if wallpaper { 0.91 } else { 1.0 }", code)

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
