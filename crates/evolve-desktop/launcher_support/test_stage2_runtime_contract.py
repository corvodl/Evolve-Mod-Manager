"""Contract tests for the October 15 Stage2Trace build. No Windows/game required."""
import hashlib
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import stage2_runtime_contract as contract

OBSERVED_MODES=(
    "Stage2ArbitrationComplete,Stage2ArbitrationTrace,Stage2Listener,"
    "Stage2LobbyTaskDataBridge,Stage2LobbyTaskMap,Stage2OwnableCompat,"
    "Stage2OwnableVtableCompat,Stage2ProfileCompat,Stage2ProfileOnly,"
    "Stage3GameContextTrace,Stage3NativeListen,Stage3RouteObjectTrace"
)

class TraceContractTests(unittest.TestCase):
    def test_game_runtime_contract_is_structure_based(self):
        with tempfile.TemporaryDirectory() as folder:
            binary=Path(folder)/"bin64_SteamRetail"
            binary.mkdir()
            exe=binary/"Evolve.exe"
            exe.write_bytes(b"EXE")
            trace=binary/"Stage2Trace.dll"
            trace.write_bytes(b"not a PE")
            with self.assertRaisesRegex(RuntimeError,"native Windows DLL"):
                contract.verify_trace(exe)
            provider=binary/"ModdedEvolveSteamCompat.dll"
            provider.write_bytes(b"not a PE")
            with self.assertRaisesRegex(RuntimeError,"native Windows DLL"):
                contract.verify_steam_provider(exe)
            with mock.patch.object(contract.native_probe,"verify",side_effect=lambda path, kind: path) as verify:
                self.assertEqual(contract.verify_trace(exe),trace)
                self.assertEqual(contract.verify_steam_provider(exe),provider)
            self.assertEqual(verify.call_count,2)

    def test_real_client_modes_accept_and_incomplete_reject(self):
        self.assertEqual(contract.validate_modes(OBSERVED_MODES),OBSERVED_MODES)
        for modes in ["", "Stage2Listener,Stage2ProfileOnly", "Stage2Listener;Stage2ProfileCompat",
                      OBSERVED_MODES+",", "Stage2Listener,Stage2Listener"]:
            with self.subTest(modes=modes), self.assertRaises(RuntimeError):
                contract.validate_modes(modes)

    def test_actual_client_service_log_modes_round_trip(self):
        # Exact delimiter and order from the supplied direct-stage2-services.log.
        reported = (
            "Stage2Listener;Stage2OwnableCompat;Stage2OwnableVtableCompat;"
            "Stage2ProfileCompat;Stage2ProfileOnly;Stage2LobbyTaskMap;"
            "Stage3GameContextTrace;Stage3RouteObjectTrace;Stage3NativeListen;"
            "Stage2LobbyTaskDataBridge;Stage2ArbitrationTrace;"
            "Stage2ArbitrationComplete"
        )
        normalized = contract.validate_modes(reported)
        self.assertEqual(normalized, reported.replace(";", ","))
        self.assertEqual(contract.validate_modes(normalized), normalized)
        self.assertEqual(len(normalized.split(",")), 12)

        with self.assertRaisesRegex(RuntimeError, "mixed separators"):
            contract.validate_modes(normalized.replace(",", ";", 1))
        with self.assertRaisesRegex(RuntimeError, "malformed"):
            contract.validate_modes(reported + ";")
        with self.assertRaisesRegex(RuntimeError, "duplicates"):
            contract.validate_modes(reported + ";Stage2Listener")

    def test_provider_must_be_installed_next_to_game_binary(self):
        with tempfile.TemporaryDirectory() as folder:
            binary=Path(folder)/"bin64_SteamRetail"
            binary.mkdir()
            exe=binary/"Evolve.exe"
            exe.write_bytes(b"EXE")
            with self.assertRaisesRegex(RuntimeError,"Steam compatibility provider"):
                contract.verify_steam_provider(exe)
            (binary/"ModdedEvolveSteamCompat.dll").write_bytes(b"dll")
            with mock.patch.object(contract.native_probe,"verify",return_value=None):
                self.assertEqual(contract.verify_steam_provider(exe),binary/"ModdedEvolveSteamCompat.dll")

    def test_catalog_is_generated_file_not_directory(self):
        with tempfile.TemporaryDirectory() as folder:
            base=Path(folder)
            target=base/"ModdedEvolveLauncher"/"stage2"/"ownable-catalog-launch.txt"
            target.parent.mkdir(parents=True)
            with self.assertRaises(RuntimeError):
                contract.catalog_from_local_appdata(base)
            target.write_bytes(b"observed catalog test")
            self.assertEqual(contract.catalog_from_local_appdata(base),target.resolve())

    def test_hook_log_requires_provider_and_actual_mode_set(self):
        provider=Path("C:/Games/EvolveGame/bin64_SteamRetail/ModdedEvolveSteamCompat.dll")
        success=(
            "[01:00:00] Stage2Tracer starting; listener_mode=enabled\n"
            "[01:00:00] STEAM_EMU active provider="+str(provider)+"\n"
            "[01:00:00] Modes from MODDED_EVOLVE_STAGE2_MODES: "+OBSERVED_MODES+"\n"
            "[01:00:00] Hooks enabled status=MH_OK\n"
        )
        contract.validate_hook_startup(success,provider,OBSERVED_MODES)
        for item in ("Stage2Tracer starting;", "STEAM_EMU active provider=",
                     "Modes from MODDED_EVOLVE_STAGE2_MODES:", "Hooks enabled status=MH_OK"):
            line=next(line for line in success.splitlines(True) if item in line)
            with self.subTest(item=item), self.assertRaises(RuntimeError):
                contract.validate_hook_startup(success.replace(line,""),provider,OBSERVED_MODES)
        with self.assertRaises(RuntimeError):
            contract.validate_hook_startup(success,Path("C:/Bad/ModdedEvolveSteamCompat.dll"),OBSERVED_MODES)

    def test_hook_startup_accepts_reordered_modes_not_changed_members(self):
        provider = Path("C:/Games/EvolveGame/bin64_SteamRetail/ModdedEvolveSteamCompat.dll")
        # The client reports these 12 modes in a different order than the
        # alphabetically sorted Stage2Trace log.
        client_modes = ";".join(reversed(OBSERVED_MODES.split(",")))
        self.assertNotEqual(contract.validate_modes(client_modes), OBSERVED_MODES)
        prefix = (
            "[01:00:00] Stage2Tracer starting; listener_mode=enabled\n"
            "[01:00:00] STEAM_EMU active provider=" + str(provider) + "\n"
        )
        mode_line = "[01:00:00] Modes from MODDED_EVOLVE_STAGE2_MODES: "
        suffix = "[01:00:00] Hooks enabled status=MH_OK\n"
        log = prefix + mode_line + OBSERVED_MODES + "\n" + suffix
        contract.validate_hook_startup(log, provider, client_modes)
        # The complete startup wait should also return, rather than timing out.
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "hook.log"
            path.write_text(log, encoding="utf-8")
            process = mock.Mock()
            process.poll.return_value = None
            contract.wait_for_hook_startup(path, provider, client_modes, process, timeout=0.5)
        observed = OBSERVED_MODES.split(",")
        invalid_lists = (
            observed[:-1],                                # missing mode
            observed + ["Stage2UnexpectedMode"],           # unexpected mode
            observed + [observed[0]],                      # duplicate mode
        )
        for members in invalid_lists:
            with self.subTest(members=members), self.assertRaises(RuntimeError):
                contract.validate_hook_startup(
                    prefix + mode_line + ",".join(members) + "\n" + suffix,
                    provider, client_modes,
                )
        with self.assertRaisesRegex(RuntimeError, "malformed"):
            contract.validate_hook_startup(
                prefix + mode_line + OBSERVED_MODES + ",bad name\n" + suffix,
                provider, client_modes,
            )
        # An earlier valid line cannot mask a later contradictory log record.
        with self.assertRaises(RuntimeError):
            contract.validate_hook_startup(
                log + mode_line + ",".join(observed[:-1]) + "\n",
                provider, client_modes,
            )

    def test_missing_or_empty_runtime_field_is_never_faked(self):
        for data in [
            {"MODDED_EVOLVE_RELAY_TOKEN":""},
            {"MODDED_EVOLVE_UNKNOWN_FIELD":"x"},
            {"MODDED_EVOLVE_LOCAL_IDENTITY":"a\nb"},
            {"MODDED_EVOLVE_RELAY":"\0"},
        ]:
            with self.assertRaises(RuntimeError):
                contract.validated_optional_session_environment(data)
        self.assertEqual(contract.validated_optional_session_environment(None),{})

    def test_complete_environment_uses_exact_local_provider_and_catalog(self):
        with tempfile.TemporaryDirectory() as folder:
            base=Path(folder)
            binary=base/"bin64_SteamRetail"
            binary.mkdir()
            exe=binary/"Evolve.exe"
            exe.write_bytes(b"EXE")
            trace=binary/"Stage2Trace.dll"
            trace.write_bytes(b"TRACE FIXTURE")
            provider=binary/"ModdedEvolveSteamCompat.dll"
            provider.write_bytes(b"PROVIDER")
            catalog=base/"ModdedEvolveLauncher"/"stage2"/"ownable-catalog-launch.txt"
            catalog.parent.mkdir(parents=True)
            catalog.write_text("catalog data")
            state={
                "status":"ready", "game_exe":str(exe),"trace":str(trace),
                "steam_compat":str(provider),"local_appdata":str(base),
                "modes":OBSERVED_MODES,
            }
            with mock.patch.object(contract.native_probe,"verify",return_value=None):
                env=contract.build_game_environment(state,base)
                self.assertEqual(env["MODDED_EVOLVE_STEAM_EMU_DLL"],str(provider))
                self.assertEqual(env["MODDED_EVOLVE_STAGE2_CATALOG"],str(catalog))
                self.assertEqual(env["MODDED_EVOLVE_STAGE2_MODES"],OBSERVED_MODES)
                self.assertNotIn("MODDED_EVOLVE_LOCAL_IDENTITY",env)
                self.assertNotIn("MODDED_EVOLVE_RELAY",env)
                self.assertNotIn("MODDED_EVOLVE_RELAY_TOKEN",env)
            state["status"]="not-ready"
            with self.assertRaisesRegex(RuntimeError,"not completed"):
                contract.build_game_environment(state,base)

if __name__=="__main__":
    unittest.main()
