"""Static and fail-closed tests for the medium-integrity Windows process broker.

CI has no interactive Explorer session; live Win32 token behavior must be
verified manually on the user's normal desktop.
"""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock


MODULE = Path(__file__).with_name("standard_user_launch.py")
spec = importlib.util.spec_from_file_location("standard_user_launch", MODULE)
broker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(broker)


class LaunchBrokerTests(unittest.TestCase):
    def test_wrong_executable_never_spawns(self):
        with tempfile.TemporaryDirectory() as folder:
            executable = Path(folder, "Evolve.exe")
            executable.write_bytes(b"not a launcher")
            with mock.patch.object(broker, "_win", side_effect=RuntimeError("no token available")):
                with self.assertRaisesRegex(ValueError, "Only a configured"):
                    broker.launch_standard_user(executable)

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

    def test_cli_fails_if_two_commands_supplied(self):
        with self.assertRaises(SystemExit):
            broker.main(["--launch", "x.exe", "--diagnose-processes"])


if __name__ == "__main__":
    unittest.main()
