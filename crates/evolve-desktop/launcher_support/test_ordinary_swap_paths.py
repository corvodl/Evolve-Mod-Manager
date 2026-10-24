"""Ordinary Win32 path parity for new and existing Stage 2 signing stages."""
import unittest
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


if __name__ == "__main__":
    unittest.main()
