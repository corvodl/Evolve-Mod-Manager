"""Direct-launch regression tests. These never execute the real client or Evolve."""
import sys
import tempfile
import json
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0,str(Path(__file__).resolve().parent))
import standard_user_launch as std
sys.modules["direct_standard_user_launch"]=std
import direct_game_launch as direct
import client_service_host as service
import managed_service_adapter as managed_adapter


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
                 mock.patch.object(direct,"wait_for_visible_game_window",side_effect=lambda proc: (self.assertEqual((session/"game.pid").read_text().strip(), str(proc.pid)), actions.append("window"))), \
                 mock.patch.object(direct,"validate_prepared",side_effect=checked), \
                 mock.patch.object(direct,"rename_prepared_into_game",side_effect=swap), \
                 mock.patch.object(direct,"start_restore_watchdog",side_effect=watchdog), \
                 mock.patch.object(direct.standard_user_launch,"launch_standard_user",side_effect=spawn):
                self.assertEqual(direct.launch_direct(root,root,"client",root),0)
            self.assertEqual(actions,["verify","services","swap","watchdog","inject","window","close"])
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
                 mock.patch.object(direct,"wait_for_visible_game_window"), \
                 mock.patch.object(direct.swapper,"all_closed"), \
                 mock.patch.object(direct,"rename_prepared_into_game") as rename, \
                 mock.patch.object(direct,"start_restore_watchdog") as watch, \
                 mock.patch.object(direct.standard_user_launch,"launch_standard_user",return_value=Game()):
                self.assertEqual(direct.launch_direct(root,None,"client",root,True),0)
            rename.assert_not_called()
            watch.assert_not_called()

    def test_hook_success_does_not_hide_early_game_exit(self):
        process = mock.Mock(pid=19328)
        process.poll.return_value = 0xC0000005
        with mock.patch.object(direct,"has_visible_game_window") as visible:
            with self.assertRaisesRegex(RuntimeError, "exit code 3221225477"):
                direct.wait_for_visible_game_window(process, timeout=1)
        visible.assert_not_called()

    def test_visible_window_must_be_stable(self):
        process = mock.Mock(pid=19328)
        process.poll.return_value = None
        with mock.patch.object(direct,"has_visible_game_window",
                               side_effect=[False, True, True]) as window, \
             mock.patch.object(direct.time,"monotonic",
                               side_effect=[0,0,0,.1,.2,.4,.6]), \
             mock.patch.object(direct.time,"sleep"):
            direct.wait_for_visible_game_window(process, timeout=1, stable_seconds=.35)
        self.assertEqual(window.call_count,3)

    def test_live_but_invisible_game_never_reports_success(self):
        process = mock.Mock(pid=19328)
        process.poll.return_value = None
        clock = iter(i * .25 for i in range(100))
        with mock.patch.object(direct,"has_visible_game_window",return_value=False), \
             mock.patch.object(direct.time,"monotonic",side_effect=lambda: next(clock)), \
             mock.patch.object(direct.time,"sleep"):
            with self.assertRaisesRegex(RuntimeError, "never displayed"):
                direct.wait_for_visible_game_window(process, timeout=1)

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

    def test_http_sys_service_ownership_does_not_block_live_testing(self):
        import client_service_host
        with tempfile.TemporaryDirectory() as folder:
            log=Path(folder)/"stage2.log"
            log.write_text(
                "STAGE2_SERVICES_STARTED listener loopback 2000/2040\n"
                "SESSION_IDENTITY_STAGE2_READY\n", encoding="utf-8")
            class Host:
                pid=8765
                def poll(self):
                    return None
            with mock.patch.object(client_service_host,"listener_owner",return_value=False), \
             mock.patch.object(client_service_host,"frontend_accepting_connections",return_value=True):
                client_service_host.wait_services(Host(),log,timeout=1)

    def test_unreachable_frontend_does_not_accept_service_log_as_ready(self):
        with tempfile.TemporaryDirectory() as folder:
            log=Path(folder)/"services.log"
            log.write_text(
                "STAGE2_SERVICES_STARTED listener loopback 2000/2040\n"
                "SESSION_IDENTITY_STAGE2_READY\n", encoding="utf-8")
            class Host:
                pid=8765
                def poll(self): return None
            with mock.patch.object(service,"listener_owner",return_value=False), \
                 mock.patch.object(service,"frontend_accepting_connections",return_value=False), \
                 mock.patch.object(service.time,"sleep"):
                with self.assertRaisesRegex(RuntimeError,"127.0.0.1:2000"):
                    service.wait_services(Host(),log,timeout=.002)

    def test_frontend_connectivity_confirms_service_readiness(self):
        with tempfile.TemporaryDirectory() as folder:
            log=Path(folder)/"services.log"
            log.write_text(
                "STAGE2_SERVICES_STARTED listener loopback 2000/2040\n"
                "SESSION_IDENTITY_STAGE2_READY\n", encoding="utf-8")
            class Host:
                pid=8765
                def poll(self): return None
            with mock.patch.object(service,"listener_owner",return_value=False), \
                 mock.patch.object(service,"frontend_accepting_connections",return_value=True):
                service.wait_services(Host(),log,timeout=1)

    def test_metadata_adapter_rejects_nonmanaged_files(self):
        with self.assertRaises(managed_adapter.UnsupportedClient):
            managed_adapter.resolve(b"not a Windows DLL")
        with self.assertRaises(managed_adapter.UnsupportedClient):
            managed_adapter.resolve(b"MZ" + bytes(128))

    def test_runtime_service_patch_resolves_tokens_instead_of_hashes(self):
        message = 0x70012345
        logger = 0x06000456
        sleeper = 0x0A000789
        prefix = (b"\x72" + message.to_bytes(4, "little")
                  + b"\x28" + logger.to_bytes(4, "little")
                  + b"\x16\x13\x14\xdd")
        source = bytearray(bytes(16) + prefix + (10).to_bytes(4, "little")
                           + bytes(60))
        class FakeMetadata:
            def __init__(self, data): pass
            def stage2_run(self): return (12345, logger & 0xFFFFFF)
            def stop_token(self): return message
            def sleep_token(self): return sleeper
            def code(self, rva): return (16, len(source) - 16)
        with mock.patch.object(managed_adapter, "_ManagedMetadata", FakeMetadata):
            offset, expected, replacement = managed_adapter.resolve(source)
            self.assertEqual(offset, 16)
            self.assertEqual(expected, source[16:34])
            self.assertEqual(replacement[6:10], sleeper.to_bytes(4,"little"))
            self.assertEqual(len(replacement),18)
            source[23] = 0xFF
            with self.assertRaisesRegex(managed_adapter.UnsupportedClient, "changed"):
                managed_adapter.resolve(source)

    def test_unknown_client_build_produces_runtime_error_report(self):
        import direct_runtime_supervisor as supervisor
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            with mock.patch.object(supervisor.client_service_host, "require_client_files",
                                   side_effect=RuntimeError("Unsupported installed client version")), \
                 mock.patch.object(supervisor.contract, "verify_trace") as trace, \
                 mock.patch.object(supervisor.client_service_host, "start_services") as services:
                with self.assertRaisesRegex(RuntimeError, "Unsupported installed client version"):
                    supervisor.supervise("client.exe", "Evolve.exe", str(root), str(root))
            self.assertIn("Unsupported installed client version",
                          json.loads((root/"error.json").read_text())["error"])
            self.assertTrue((root/"stopped.json").is_file())
            trace.assert_not_called()
            services.assert_not_called()

    def test_unknown_client_layout_rejected_without_modifying_source(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            launcher=root/"ModdedEvolveLauncher.exe"
            launcher.write_bytes(b"launcher")
            for relative in ("ModdedEvolveLauncher.dll",
                             "Compat/ModdedEvolveSteamCompat.dll",
                             "Compat/Stage2/Stage2Trace.dll"):
                target=root/relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(b"runtime")
            with self.assertRaisesRegex(RuntimeError, "Windows DLL"):
                service.require_client_files(launcher)
            self.assertEqual((root/"ModdedEvolveLauncher.dll").read_bytes(),b"runtime")

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
