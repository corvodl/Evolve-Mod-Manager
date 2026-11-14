import importlib.util
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET


SCRIPT = Path(__file__).resolve().parents[1] / "scripts/experiments/build_throwable_dome_probe.py"
spec = importlib.util.spec_from_file_location("dome_probe", SCRIPT)
dome_probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dome_probe)


class DomeProbeTest(unittest.TestCase):
    def test_build_preserves_shared_world_and_uses_existing_throw_mode(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "source.xml"
            source.write_text('''<item name="MobileArenaTriggerWeapon" class="Weapon"><params><param name="selectable" value="0"/></params><ammos><ammo name="MobileArenaSharedWorld"/></ammos><firemodes><firemode type="MobileArenaTrigger" name="Standard"/></firemodes></item>''')
            output = Path(tmp) / "out.xml"
            dome_probe.build(source, output)
            root = ET.parse(output).getroot()
            self.assertEqual(root.find("firemodes/firemode").get("type"), "Throw")
            self.assertEqual(root.find("firemodes/firemode/fire/param[@name='ammo_type']").get("value"), "MobileArenaAmmo")
            self.assertEqual(root.find("params/param[@name='selectable']").get("value"), "1")
            self.assertEqual([a.get("name") for a in root.findall("ammos/ammo")], ["MobileArenaSharedWorld", "MobileArenaAmmo"])
            with self.assertRaises(ValueError):
                dome_probe.build(source, output)

    def test_rejects_unknown_weapon(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "source.xml"
            source.write_text('<item name="OtherWeapon" class="Weapon"/>')
            with self.assertRaises(ValueError):
                dome_probe.build(source, Path(tmp) / "out.xml")


if __name__ == "__main__":
    unittest.main()
