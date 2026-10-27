"""Offline direct-game regression checks. Never launch the real Evolve binary."""
import os
import sys
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import direct_game_launch as direct


class DirectGameLaunchTests(unittest.TestCase):
    def test_missing_executable_aborts_before_swap(self):
        with mock.patch.object(direct, "_regular", return_value=False), \
             mock.patch.object(direct, "rename_prepared_into_game") as swapping:
            with self.assertRaisesRegex(RuntimeError, "Evolve.exe"):
                direct.launch_direct("not-real", "not-real-stage")
            swapping.assert_not_called()

    def test_verified_swap_and_watchdog_precede_game_process(self):
        actions = []
        state = {"version": 1, "phase": "prepared", "files": []}
        class FakeProcess:
            pid = 42
            def close(self):
                actions.append("close_handle")

        def checked(game, stage):
            actions.append("verify")
            return state

        def swap(verified):
            self.assertIs(verified, state)
            actions.append("swap")

        def watchdog():
            actions.append("watchdog")
            return 51

        def launch(executable, *, game):
            self.assertTrue(game)
            self.assertEqual(executable.name.lower(), "evolve.exe")
            actions.append("game")
            return FakeProcess()

        with mock.patch.object(direct, "_regular", return_value=True), \
             mock.patch.object(direct, "validate_prepared", side_effect=checked), \
             mock.patch.object(direct, "rename_prepared_into_game", side_effect=swap), \
             mock.patch.object(direct, "start_restore_watchdog", side_effect=watchdog), \
             mock.patch.object(direct.standard_user_launch, "launch_standard_user", side_effect=launch):
            self.assertEqual(direct.launch_direct("C:/Game", "C:/Stage"), 0)
        self.assertEqual(actions, ["verify", "swap", "watchdog", "game", "close_handle"])

    def test_rename_records_swapping_before_any_mutation(self):
        events = []
        state = {"version": 1, "phase": "prepared", "files": [
            {"target": "original.pak", "backup": "original.pak.customkey-original",
             "ready": "original.pak.customkey-ready"}
        ]}
        with mock.patch.object(direct.swapper, "no_game", side_effect=lambda: events.append("no_game")), \
             mock.patch.object(direct.swapper, "write_state",
                               side_effect=lambda row: events.append(row["phase"])), \
             mock.patch.object(direct.os, "replace", side_effect=lambda *args: events.append("rename")):
            direct.rename_prepared_into_game(state)
        self.assertEqual(events, ["no_game", "swapping", "rename", "rename", "swapped"])

    def test_partial_rename_does_not_claim_swapped(self):
        state = {"version": 1, "phase": "prepared", "files": [
            {"target": "original.pak", "backup": "backup", "ready": "ready"}
        ]}
        states = []
        with mock.patch.object(direct.swapper, "no_game"), \
             mock.patch.object(direct.swapper, "write_state",
                               side_effect=lambda row: states.append(row["phase"])), \
             mock.patch.object(direct.os, "replace", side_effect=OSError("disk failure")):
            with self.assertRaisesRegex(OSError, "disk failure"):
                direct.rename_prepared_into_game(state)
        self.assertEqual(states, ["swapping", "interrupted"])

    def test_process_creation_failure_keeps_watchdog_running(self):
        with mock.patch.object(direct, "_regular", return_value=True), \
             mock.patch.object(direct, "validate_prepared", return_value={"files":[]}), \
             mock.patch.object(direct, "rename_prepared_into_game"), \
             mock.patch.object(direct, "start_restore_watchdog", return_value=51), \
             mock.patch.object(direct.standard_user_launch, "launch_standard_user",
                               side_effect=RuntimeError("unable to start")):
            with self.assertRaisesRegex(RuntimeError, "watchdog.*51"):
                direct.launch_direct("C:/Game", "C:/Stage")


if __name__ == "__main__":
    unittest.main()
