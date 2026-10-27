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
    def test_new_dll_is_pinned_to_uploaded_october_build(self):
        self.assertEqual(contract.STAGE2_TRACE_SHA256,
                         "a0219d94b3c7ec4d73999b531adbbf13aa595bf39babc9e7810a1cf6db0d1067")
        self.assertEqual(contract.STAGE2_TRACE_SIZE, 2217472)
        self.assertNotEqual(contract.STAGE2_TRACE_SHA256,
                            "e4a1c81e1970de697b271b3756f79040b82633fe4569c3a65bb20787cfa5be5c")

    def test_real_client_modes_accept_and_incomplete_reject(self):
        self.assertEqual(contract.validate_modes(OBSERVED_MODES),OBSERVED_MODES)
        for modes in ["", "Stage2Listener,Stage2ProfileOnly", "Stage2Listener;Stage2ProfileCompat",
                      OBSERVED_MODES+",", "Stage2Listener,Stage2Listener"]:
            with self.subTest(modes=modes), self.assertRaises(RuntimeError):
                contract.validate_modes(modes)

    def test_provider_must_be_installed_next_to_game_binary(self):
        with tempfile.TemporaryDirectory() as folder:
            binary=Path(folder)/"bin64_SteamRetail"
            binary.mkdir()
            exe=binary/"Evolve.exe"
            exe.write_bytes(b"EXE")
            with self.assertRaisesRegex(RuntimeError,"Steam compatibility provider"):
                contract.verify_steam_provider(exe)
            (binary/"ModdedEvolveSteamCompat.dll").write_bytes(b"dll")
            with mock.patch.object(contract,"_sha256",return_value=next(iter(contract.STEAM_COMPAT_SHA256))):
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
            trace.write_bytes(b"\0"*contract.STAGE2_TRACE_SIZE)
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
            def digest(path):
                return contract.STAGE2_TRACE_SHA256 if Path(path).name=="Stage2Trace.dll" else next(iter(contract.STEAM_COMPAT_SHA256))
            with mock.patch.object(contract,"_sha256",side_effect=digest):
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
