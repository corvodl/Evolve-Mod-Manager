"""Regression checks for source-authored FBX mesh naming (no Blender required)."""
import json
import struct
import tempfile
import unittest
from pathlib import Path
from blender_export_hierarchy import clean_name, original_part_labels


def glb_for(names):
    doc = {"asset": {"version": "2.0"},
           "materials": [{"name": n} for n in names]}
    data = json.dumps(doc).encode("utf-8")
    return (b"glTF" + struct.pack("<II", 2, 20 + len(data)) +
            struct.pack("<I", len(data)) + b"JSON" + data)


class HierarchyTests(unittest.TestCase):
    def test_cdf_material_identity_restores_attachment_name(self):
        names = [
            "CryEngine slot 000 - CDF part 000 body | original ID 000 | armor",
            "CryEngine slot 001 - CDF part 001 left arm | original ID 000 | glove",
            "CryEngine slot 002 - CDF part 001 left arm | original ID 001 | glove trim",
            "CryEngine slot 003 - Plain standalone material",
        ]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "character.glb"
            path.write_bytes(glb_for(names))
            self.assertEqual(original_part_labels(path),
                             {0: "body", 1: "left arm", 2: "left arm", 3: None})

    def test_labels_do_not_accept_path_or_control_characters(self):
        self.assertEqual(clean_name("../../Body\nWeapon"), "Body_Weapon")
        self.assertEqual(clean_name("  "), "Part")


if __name__ == "__main__":
    unittest.main()
