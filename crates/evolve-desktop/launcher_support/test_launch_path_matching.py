"""Regression coverage for Test B's missed CreateProcessW swap interception."""
from pathlib import Path
from unittest import mock
import unittest

import auto_launch_frida as launcher


DRIVE = r"C:\Games\ModdedEvolve\EvolveGame\bin64_SteamRetail\Evolve.exe"
EXTENDED_DRIVE = "\\\\?\\" + DRIVE
UNC = r"\\server\share\EvolveGame\bin64_SteamRetail\Evolve.exe"
EXTENDED_UNC = "\\\\?\\UNC\\" + UNC[2:]


class LaunchPathMatchingTests(unittest.TestCase):
    def setUp(self):
        self.original = launcher.GAME_EXE

    def tearDown(self):
        launcher.GAME_EXE = self.original

    def check_both(self, expected, candidate):
        launcher.GAME_EXE = Path(expected)
        self.assertTrue(launcher.normalize_exec_command(candidate), (expected, candidate))
        self.assertTrue(launcher.is_evolve_launch("", candidate))

    def test_regular_launch_matches_extended_journal_path(self):
        self.check_both(EXTENDED_DRIVE, DRIVE)
        self.check_both(EXTENDED_DRIVE, '"' + DRIVE + '" -log')
        self.check_both(DRIVE, EXTENDED_DRIVE)
        self.check_both(DRIVE, '"' + EXTENDED_DRIVE + '" /offline')

    def test_drive_case_and_slash_spelling(self):
        self.check_both(EXTENDED_DRIVE, DRIVE.upper().replace("\\", "/"))
        self.check_both(EXTENDED_DRIVE, '  "' + DRIVE.upper() + '"  -log  ')

    def test_extended_unc_preserves_network_root(self):
        expected = launcher.canonical_exec_path(UNC)
        self.assertTrue(expected.startswith("\\\\server\\share\\"))
        self.assertEqual(expected, launcher.canonical_exec_path(EXTENDED_UNC))
        self.check_both(EXTENDED_UNC, UNC)
        self.check_both(UNC, EXTENDED_UNC)
        self.check_both(EXTENDED_UNC, '"' + UNC + '" -log')

    def test_full_client_port_snapshot_checks_only_loopback_services(self):
        calls = []
        class Connected:
            def __enter__(self): return self
            def __exit__(self, *_): return None
        def probe(address, timeout):
            calls.append((address, timeout))
            if address[1] == 2002:
                raise ConnectionRefusedError()
            return Connected()
        with mock.patch.object(launcher.socket, "create_connection", side_effect=probe):
            result = launcher.probe_full_client_services()
        self.assertEqual(result, {"frontend": True, "friends_overlay": False})
        self.assertEqual(calls, [
            (("127.0.0.1", 2000), 0.3),
            (("127.0.0.1", 2002), 0.3),
        ])

    def test_rejects_other_executables_or_non_boundary_suffixes(self):
        launcher.GAME_EXE = Path(EXTENDED_DRIVE)
        for value in (
            DRIVE.replace("Evolve.exe", "Other.exe"),
            DRIVE + "evil",
            DRIVE + r"\child.exe",
            '"' + DRIVE + '"evil',
            '"' + DRIVE + "evil" + '"',
            '"' + DRIVE,
            "",
        ):
            with self.subTest(value=value):
                self.assertFalse(launcher.normalize_exec_command(value))
        self.assertFalse(launcher.is_evolve_launch(DRIVE.replace("Evolve.exe", "Other.exe"), '"' + DRIVE + '"'))


if __name__ == "__main__":
    unittest.main()
