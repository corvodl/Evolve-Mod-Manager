"""Direct-launch regression tests. These never execute the real client or Evolve."""
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0,str(Path(__file__).resolve().parent))
import standard_user_launch as std
sys.modules["direct_standard_user_launch"]=std
import direct_game_launch as direct
import client_service_host as service


class DirectGameTests(unittest.TestCase):
    def test_no_game_file_never_starts_services_or_swaps(self):
        with mock.patch.object(direct,"_regular",return_value=False), \
             mock.patch.object(direct,"start_runtime_supervisor") as start, \
             mock.patch.object(direct,"rename_prepared_into_game") as swap:
            with self.assertRaisesRegex(RuntimeError,"Evolve.exe"):
                direct.launch_direct("C:/missing","C:/stage","C:/client.exe","C:/swap")
            start.assert_not_called()
            swap.assert_not_called()

    def test_service_host_is_ready_before_modded_swap_or_injection(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            session=root/"runtime"
            session.mkdir()
            actions=[]
            state={
                "trace":str(root/"Stage2Trace.dll"),
                "steam_compat":str(root/"ModdedEvolveSteamCompat.dll"),
                "modes":"Stage2Listener;Stage2ProfileCompat",
                "catalog":str(root/"catalog"),
            }
            class Game:
                pid=12345
                def close(self):
                    actions.append("close")
            def service_start(*args):
                actions.append("services")
                return session,state
            def checked(*args):
                actions.append("verify")
                return {"phase":"prepared","files":[]}
            def swap(_):
                actions.append("swap")
            def watchdog():
                actions.append("watchdog")
                return 900
            def spawn(*args,**kwargs):
                self.assertEqual(kwargs["trace_dll"],state["trace"])
                self.assertEqual(kwargs["runtime_env"]["MODDED_EVOLVE_STAGE2_MODES"],state["modes"])
                actions.append("inject")
                return Game()
            with mock.patch.object(direct,"_ordinary",return_value=root), \
                 mock.patch.object(direct,"_regular",return_value=True), \
                 mock.patch.object(direct,"start_runtime_supervisor",side_effect=service_start), \
                 mock.patch.object(direct,"environment_for_session",return_value={"MODDED_EVOLVE_STAGE2_MODES":state["modes"],"MODDED_EVOLVE_STAGE2_LOG":str(root/"trace.log")}), \
                 mock.patch.object(direct.contract,"wait_for_hook_startup"), \
                 mock.patch.object(direct,"validate_prepared",side_effect=checked), \
                 mock.patch.object(direct,"rename_prepared_into_game",side_effect=swap), \
                 mock.patch.object(direct,"start_restore_watchdog",side_effect=watchdog), \
                 mock.patch.object(direct.standard_user_launch,"launch_standard_user",side_effect=spawn):
                self.assertEqual(direct.launch_direct(root,root,"client",root),0)
            self.assertEqual(actions,["verify","services","swap","watchdog","inject","close"])
            self.assertEqual((session/"game.pid").read_text().strip(),"12345")

    def test_clean_launch_never_swaps_or_starts_watchdog(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            session=root/"session"
            session.mkdir()
            state={"trace":"trace.dll","steam_compat":"steam.dll","modes":"X","catalog":"catalog"}
            class Game:
                pid=42
                def close(self):pass
            with mock.patch.object(direct,"_ordinary",return_value=root), \
                 mock.patch.object(direct,"_regular",return_value=True), \
                 mock.patch.object(direct,"start_runtime_supervisor",return_value=(session,state)), \
                 mock.patch.object(direct,"environment_for_session",return_value={"MODDED_EVOLVE_STAGE2_MODES":"X","MODDED_EVOLVE_STAGE2_LOG":str(root/"trace.log")}), \
                 mock.patch.object(direct.contract,"wait_for_hook_startup"), \
                 mock.patch.object(direct.swapper,"all_closed"), \
                 mock.patch.object(direct,"rename_prepared_into_game") as rename, \
                 mock.patch.object(direct,"start_restore_watchdog") as watch, \
                 mock.patch.object(direct.standard_user_launch,"launch_standard_user",return_value=Game()):
                self.assertEqual(direct.launch_direct(root,None,"client",root,True),0)
            rename.assert_not_called()
            watch.assert_not_called()

    def test_swap_journal_is_written_before_renames_and_interrupted_on_error(self):
        state={"phase":"prepared","files":[{"target":"live","backup":"backup","ready":"prepared"}]}
        states=[]
        with mock.patch.object(direct.swapper,"no_game"), \
             mock.patch.object(direct.swapper,"write_state",
                 side_effect=lambda item:states.append(item["phase"])), \
             mock.patch.object(direct.os,"replace",side_effect=OSError("rename failed")):
            with self.assertRaisesRegex(OSError,"rename failed"):
                direct.rename_prepared_into_game(state)
        self.assertEqual(states,["swapping","interrupted"])

    def test_partial_prelaunch_swap_stops_own_service_then_restores(self):
        import reusable_restore
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            session=root/"session"
            session.mkdir()
            state={"trace":"trace.dll","steam_compat":"compat.dll",
                   "modes":"Stage2Listener,Stage2ProfileCompat,Stage2ProfileOnly"}
            order=[]
            def interrupt(_):
                order.append("partial_swap")
                raise OSError("simulated rename failure")
            def shut_down(_session):
                self.assertEqual(_session,session)
                order.append("host_closed")
            def restore():
                order.append("restore_originals")
                return 0
            with mock.patch.object(direct,"_ordinary",return_value=root), \
                 mock.patch.object(direct,"_regular",return_value=True), \
                 mock.patch.object(direct,"validate_prepared",return_value={"files":[]}), \
                 mock.patch.object(direct,"start_runtime_supervisor",return_value=(session,state)), \
                 mock.patch.object(direct,"environment_for_session",return_value={"MODDED_EVOLVE_STAGE2_LOG":str(root/"hook.log")}), \
                 mock.patch.object(direct,"rename_prepared_into_game",side_effect=interrupt), \
                 mock.patch.object(direct,"stop_runtime_before_restore",side_effect=shut_down), \
                 mock.patch.object(direct,"start_restore_watchdog") as watchdog, \
                 mock.patch.object(reusable_restore,"main",side_effect=restore):
                with self.assertRaisesRegex(OSError,"simulated rename failure"):
                    direct.launch_direct(root,root,"client",root)
            self.assertEqual(order,["partial_swap","host_closed","restore_originals"])
            watchdog.assert_not_called()

    def test_pinned_shadow_patch_has_fixed_length_and_never_changes_original(self):
        self.assertEqual(len(service.PATCH_EXPECTED),len(service.PATCH_REPLACEMENT))
        self.assertEqual(len(service.PATCH_REPLACEMENT),18)
        self.assertEqual(len(service.CLIENT_DIGEST),64)

    def test_session_environment_calls_verified_contract(self):
        import stage2_runtime_contract as contract
        with mock.patch.object(contract,"build_game_environment",return_value={
            "MODDED_EVOLVE_STAGE2_MODES":"Stage2Listener,Stage2ProfileCompat,Stage2ProfileOnly"
        }) as builder:
            state={"status":"ready"}
            environment=direct.environment_for_session(state,Path("C:/swap"))
        builder.assert_called_once_with(state,Path("C:/swap"))
        self.assertNotIn("MODDED_EVOLVE_RELAY_TOKEN",environment)


if __name__=="__main__":
    unittest.main()
