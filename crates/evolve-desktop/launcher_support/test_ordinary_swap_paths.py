"""Ordinary Win32 path parity for new and existing Stage 2 signing stages."""
import unittest
import tempfile
from pathlib import Path
import controlled_swap as swap


class OrdinaryGameRootTests(unittest.TestCase):
    def test_strip_extended_drive_prefix(self):
        old = r"\\?\C:\Games\ModdedEvolve\EvolveGame"
        normal = r"C:\Games\ModdedEvolve\EvolveGame"
        self.assertEqual(swap.ordinary_windows_path(old), normal)
        self.assertTrue(swap.same_game_root(old, normal))
        self.assertTrue(swap.same_game_root(old, "c:/GAMES/ModdedEvolve/EvolveGame"))

    def test_keep_unc_root(self):
        old = r"\\?\UNC\server\share\EvolveGame"
        normal = r"\\server\share\EvolveGame"
        self.assertEqual(swap.ordinary_windows_path(old), normal)
        self.assertTrue(swap.same_game_root(normal, old))

    def test_different_game_path_not_accepted(self):
        expected = r"\\?\C:\Games\ModdedEvolve\EvolveGame"
        self.assertFalse(swap.same_game_root(expected, r"D:\Games\ModdedEvolve\EvolveGame"))
        self.assertFalse(swap.same_game_root(expected, r"C:\Games\Other\EvolveGame"))

    def test_ordinary_path_stays_ordinary(self):
        original = "C:/Games/ModdedEvolve/EvolveGame"
        self.assertEqual(swap.ordinary_windows_path(original), original)



class AbandonedPartialTests(unittest.TestCase):
    def setUp(self):
        root = tempfile.TemporaryDirectory()
        self.addCleanup(root.cleanup)
        self.folder = Path(root.name)
        self.target = self.folder / "objects_min_level_specific.pak"
        self.ready = self.folder / "objects_min_level_specific.pak.customkey-ready"
        self.backup = self.folder / "objects_min_level_specific.pak.customkey-original"
        self.partial = Path(str(self.ready) + ".partial")
        self.target.write_bytes(b"untouched original")
        self.partial.write_bytes(b"incomplete signed cache bytes")

    def test_abandoned_partial_is_preserved_then_fresh_copy_can_start(self):
        old = self.partial.read_bytes()
        stale = swap.preserve_abandoned_partial(
            self.partial, self.target, self.ready, self.backup
        )
        self.assertFalse(self.partial.exists())
        self.assertTrue(stale.name.startswith(self.partial.name + ".stale-"))
        self.assertEqual(stale.read_bytes(), old)
        self.assertEqual(self.target.read_bytes(), b"untouched original")
        self.assertFalse(self.ready.exists())
        self.assertFalse(self.backup.exists())
        # The existing prepare path then opens a brand-new .partial with xb.
        with self.partial.open("xb") as output:
            output.write(b"fresh source bytes")
        self.assertEqual(self.partial.read_bytes(), b"fresh source bytes")
        self.assertEqual(stale.read_bytes(), old)

    def test_original_backup_blocks_mutation(self):
        self.backup.write_bytes(b"recoverable original")
        with self.assertRaisesRegex(RuntimeError, "Original game backup"):
            swap.preserve_abandoned_partial(
                self.partial, self.target, self.ready, self.backup
            )
        self.assertTrue(self.partial.exists())
        self.assertEqual(self.backup.read_bytes(), b"recoverable original")

    def test_existing_prepared_copy_blocks_mutation(self):
        self.ready.write_bytes(b"previous prepared file")
        with self.assertRaisesRegex(RuntimeError, "existing prepared file"):
            swap.preserve_abandoned_partial(
                self.partial, self.target, self.ready, self.backup
            )
        self.assertTrue(self.partial.exists())
        self.assertEqual(self.ready.read_bytes(), b"previous prepared file")

    def test_directory_instead_of_partial_blocks_mutation(self):
        self.partial.unlink()
        self.partial.mkdir()
        with self.assertRaisesRegex(RuntimeError, "Unsafe abandoned prepared file"):
            swap.preserve_abandoned_partial(
                self.partial, self.target, self.ready, self.backup
            )
        self.assertTrue(self.partial.is_dir())
        self.assertEqual(self.target.read_bytes(), b"untouched original")

    def test_missing_original_blocks_mutation(self):
        self.target.unlink()
        with self.assertRaisesRegex(RuntimeError, "Installed original"):
            swap.preserve_abandoned_partial(
                self.partial, self.target, self.ready, self.backup
            )
        self.assertEqual(self.partial.read_bytes(), b"incomplete signed cache bytes")

if __name__ == "__main__":
    unittest.main()
