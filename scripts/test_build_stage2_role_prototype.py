"""Asset-free safety tests for the isolated role movie builder."""

import importlib.util
import struct
import tempfile
import unittest
from pathlib import Path


spec = importlib.util.spec_from_file_location(
    "role_builder", Path(__file__).with_name("build-stage2-role-prototype.py")
)
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


def movie(script: bytes, other: bytes = b"x") -> bytes:
    def tag(code: int, body: bytes) -> bytes:
        return struct.pack("<H", code << 6 | len(body)) + body

    body = (b"\x08\x00" + struct.pack("<HH", 30 * 256, 1)
            + tag(1, other) + tag(12, script) + tag(0, b""))
    return b"GFX\x0a" + struct.pack("<I", 8 + len(body)) + body


class RoleBuilderTests(unittest.TestCase):
    def test_only_root_script_change_is_permitted(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            stock, edited, art_changed = (root / name for name in ("stock", "edited", "art"))
            stock.write_bytes(movie(b"original"))
            edited.write_bytes(movie(b"patched"))
            art_changed.write_bytes(movie(b"patched", b"different art"))
            builder.verify_only_role_script_changed(stock, edited)
            with self.assertRaises(ValueError):
                builder.verify_only_role_script_changed(stock, stock)
            with self.assertRaises(ValueError):
                builder.verify_only_role_script_changed(stock, art_changed)

    def test_unknown_source_is_rejected_without_guessing(self):
        with self.assertRaises(ValueError):
            builder.patch_script("function openScreen() {}")


if __name__ == "__main__":
    unittest.main()
