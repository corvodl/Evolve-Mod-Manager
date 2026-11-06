"""Static and fail-closed tests for the medium-integrity Windows process broker.

CI has no interactive Explorer session; live Win32 token behavior must be
verified manually on the user's normal desktop.
"""
import importlib.util
import hashlib
from pathlib import Path
import struct
import tempfile
import unittest
from unittest import mock


MODULE = Path(__file__).with_name("standard_user_launch.py")
spec = importlib.util.spec_from_file_location("standard_user_launch", MODULE)
broker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(broker)


class LaunchBrokerTests(unittest.TestCase):
    def test_native_probe_requires_pinned_x64_pe_and_is_off_by_default(self):
        with tempfile.TemporaryDirectory() as folder:
            dll = Path(folder, "LegacyKitCompositor.dll")
            image = bytearray(1024)
            image[:2] = b"MZ"
            struct.pack_into("<I", image, 0x3c, 0x80)
            image[0x80:0x86] = b"PE\0\0\x64\x86"
            struct.pack_into("<H", image, 0x80 + 22, 0x2000)
            struct.pack_into("<H", image, 0x80 + 24, 0x20b)
            dll.write_bytes(image)
            digest = hashlib.sha256(image).hexdigest()
            self.assertEqual(broker.validate_kit_compositor(dll, digest), dll.resolve())
            with self.assertRaisesRegex(ValueError, "SHA-256"):
                broker.validate_kit_compositor(dll, "0" * 64)
            with self.assertRaisesRegex(ValueError, "explicit SHA-256"):
                broker.validate_kit_compositor(dll, None)
            with mock.patch.object(broker, "_win", side_effect=AssertionError("must not start")):
                with self.assertRaisesRegex(ValueError, "offline Evolve"):
                    broker.launch_standard_user(dll, compositor_dll=dll, compositor_sha256=digest)
                with self.assertRaisesRegex(ValueError, "requires an offline Evolve"):
                    broker.launch_standard_user(dll, game=True, compositor_dll=dll,
                                                compositor_sha256=digest)
            image[0x84:0x86] = b"\x4c\x01"
            dll.write_bytes(image)
            with self.assertRaisesRegex(ValueError, "x64"):
                broker.validate_kit_compositor(dll, hashlib.sha256(image).hexdigest())
            image[0x84:0x86] = b"\x64\x86"
            image[0x80 + 22:0x80 + 24] = b"\0\0"
            dll.write_bytes(image)
            with self.assertRaisesRegex(ValueError, "must be a DLL"):
                broker.validate_kit_compositor(dll, hashlib.sha256(image).hexdigest())

    def test_wrong_executable_never_spawns(self):
        with tempfile.TemporaryDirectory() as folder:
            executable = Path(folder, "Evolve.exe")
            executable.write_bytes(b"not a launcher")
            with mock.patch.object(broker, "_win", side_effect=RuntimeError("no token available")):
                with self.assertRaisesRegex(ValueError, "Only a configured"):
                    broker.launch_standard_user(executable)

    def test_test_client_can_have_different_executable_basename(self):
        with tempfile.TemporaryDirectory() as folder:
            executable = Path(folder, "EvolveTestClient.exe")
            executable.write_bytes(b"test")
            with mock.patch.object(broker, "_win", side_effect=RuntimeError("no interactive shell")):
                with self.assertRaisesRegex(RuntimeError, "no interactive shell"):
                    broker.launch_standard_user(executable, test_client=True)
                with self.assertRaisesRegex(ValueError, "Only a configured"):
                    broker.launch_standard_user(executable)

    def test_test_client_cli_remains_explicit(self):
        class FakeProcess:
            pid = 1234
            def close(self): pass
        with (
            mock.patch.object(broker, "launch_standard_user", return_value=FakeProcess()) as start,
            mock.patch.object(broker, "get_elevation", return_value={"session": 1}),
        ):
            self.assertEqual(broker.main(["--launch-test", "EvolveTestClient.exe"]), 0)
        start.assert_called_once_with("EvolveTestClient.exe", game=False, test_client=True)

    def test_shell_lookup_failure_does_not_fallback_to_admin(self):
        with tempfile.TemporaryDirectory() as folder:
            executable = Path(folder, "ModdedEvolveLauncher.exe")
            executable.write_bytes(b"test")
            with mock.patch.object(broker, "_win", side_effect=RuntimeError("no interactive shell")):
                with mock.patch.object(broker.subprocess, "Popen") as spawn:
                    with self.assertRaisesRegex(RuntimeError, "no interactive shell"):
                        broker.launch_standard_user(executable)
                    spawn.assert_not_called()

    def test_process_wrapper_uses_win32_handle_not_subprocess_fallback(self):
        calls = []
        def exit_code(_h, out):
            out._obj.value = broker.STILL_ACTIVE
            calls.append("poll")
            return True
        api = {
            "GetExitCodeProcess": exit_code,
            "TerminateProcess": lambda handle, code: calls.append("terminate") or True,
            "CloseHandle": lambda handle: calls.append("close") or True,
        }
        child = broker.StandardUserProcess(api, 1337, 1)
        self.assertIsNone(child.poll())
        child.terminate()
        child.close()
        self.assertEqual(calls, ["poll", "poll", "terminate", "close"])

    def test_game_environment_has_real_nul_delimiters(self):
        block = broker._windows_environment_block([
            "SteamAppId=273350",
            "SteamGameId=273350",
            "SteamOverlayGameId=273350",
            "MODDED_EVOLVE_STAGE2_FRONTEND=127.0.0.1:2000",
        ])
        chars = block[:]
        self.assertEqual(
            chars,
            "SteamAppId=273350\0SteamGameId=273350\0"
            "SteamOverlayGameId=273350\0"
            "MODDED_EVOLVE_STAGE2_FRONTEND=127.0.0.1:2000\0\0",
        )
        self.assertNotIn(chr(92) + "0", chars)
        with self.assertRaises(ValueError):
            broker._windows_environment_block(["A=valid", "B=bad\0data"])

    def test_cli_fails_if_two_commands_supplied(self):
        with self.assertRaises(SystemExit):
            broker.main(["--launch", "x.exe", "--diagnose-processes"])


if __name__ == "__main__":
    unittest.main()
