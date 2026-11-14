import importlib.util
from pathlib import Path
import struct
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "scripts/experiments/encode_native_cryxml_probe.py"
spec = importlib.util.spec_from_file_location("native_cryxml_probe", SCRIPT)
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


class NativeCryXmlProbeTest(unittest.TestCase):
    def test_shipped_stage2_table_order_and_root_string(self):
        binary = native.encode(b'<item name="MobileArenaTriggerWeapon"><firemode type="Throw"/></item>')
        size, nodes_at, nodes, attrs_at, attrs, children_at, children, strings_at, strings_len = struct.unpack_from("<9I", binary, 8)
        self.assertEqual(size, len(binary))
        self.assertEqual((nodes_at, nodes, attrs, children), (44, 2, 2, 1))
        self.assertEqual(children_at, nodes_at + nodes * 28)
        self.assertEqual(attrs_at, children_at + children * 4)
        self.assertEqual(strings_at, attrs_at + attrs * 8)
        self.assertEqual(binary[strings_at:strings_at + 6], b"item\0\0")
        self.assertEqual(struct.unpack_from("<IIHHIIII", binary, 44), (0, 5, 1, 1, 0xffffffff, 0, 0, 0))
        self.assertEqual(strings_len, len(binary) - strings_at)


if __name__ == "__main__":
    unittest.main()
