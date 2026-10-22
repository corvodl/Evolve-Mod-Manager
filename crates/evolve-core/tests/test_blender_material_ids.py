"""Source CryEngine material ID integrity without needing Blender or game files."""
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "src"))

from blender_material_ids import (
    bind_original_material_ids, parse_material_records,
    source_material_histogram, verify_original_material_ids,
)


class Material:
    def __init__(self, name):
        self.name = name
        self.props = {}

    def copy(self):
        copied = Material(self.name)
        copied.props = dict(self.props)
        return copied

    def __setitem__(self, key, value):
        self.props[key] = value

    def get(self, key, default=None):
        return self.props.get(key, default)


class Polygon:
    def __init__(self, index):
        self.material_index = index
        self.vertices = (0, 1, 2)


class MeshData:
    def __init__(self, materials, face_ids):
        self.materials = list(materials)
        self.polygons = [Polygon(index) for index in face_ids]


class Mesh:
    def __init__(self, name, materials, faces):
        self.name = name
        self.data = MeshData(materials, faces)


class SourceMaterialTests(unittest.TestCase):
    def test_sparse_2012_equipment_id_remains_ten(self):
        mesh = Mesh("historical", [
            Material("CryEngine slot 000 - Body"),
            Material("CryEngine slot 010 - NoDraw attachment"),
        ], [0, 0, 1])
        expected = [
            {"id": 0, "name": "Body", "triangles": 2},
            {"id": 10, "name": "NoDraw attachment", "triangles": 1,
             "no_draw": True},
        ]
        bind_original_material_ids([mesh], 0, "Historical Character", expected)
        self.assertEqual(source_material_histogram([mesh]), {(0, 0): 2, (0, 10): 1})
        self.assertIn("P000_S010__", mesh.data.materials[1].name)
        self.assertTrue(mesh.data.materials[1].get("evolve_source_no_draw"))
        self.assertEqual(verify_original_material_ids([mesh], [expected]),
                         {(0, 0): 2, (0, 10): 1})

    def test_faces_remain_associated_after_material_split_and_reorder(self):
        expected = [
            {"id": 2, "name": "Face", "triangles": 2},
            {"id": 11, "name": "Equip", "triangles": 1},
        ]
        original = Mesh("Emet", [
            Material("CryEngine slot 002 - Face"),
            Material("CryEngine slot 011 - Equip"),
        ], [0, 1, 0])
        bind_original_material_ids([original], 3, "Emet", expected)
        separated = [
            Mesh("Emet_material_000", [original.data.materials[1]], [0]),
            Mesh("Emet_material_001", [original.data.materials[0]], [0, 0]),
        ]
        self.assertEqual(source_material_histogram(separated),
                         {(3, 2): 2, (3, 11): 1})
        # Verify the complete global audit with intermediate parts present.
        missing = [Mesh("other" + str(i), [Material(
            "EVOLVE_MID_P%03d_S000__Body" % i)], [0])
            for i in range(3)]
        previous = [[{"id": 0, "triangles": 1, "name": "Body"}]
                    for _ in range(3)]
        verify_original_material_ids(missing + separated, previous + [expected])

    def test_wrong_material_face_count_fails_closed(self):
        original = Mesh("Emet", [
            Material("CryEngine slot 000 - Body"),
            Material("CryEngine slot 010 - Face"),
        ], [0, 0, 1])
        records = [
            {"id": 0, "name": "Body", "triangles": 2},
            {"id": 10, "name": "Face", "triangles": 1},
        ]
        bind_original_material_ids([original], 0, "Emet", records)
        original.data.polygons[1].material_index = 1
        with self.assertRaisesRegex(RuntimeError, "reassigned a source material ID"):
            verify_original_material_ids([original], [records])

    def test_missing_multi_slot_source_names_cannot_be_guessed(self):
        original = Mesh("unknown", [Material("material_001"),
                                    Material("material_002")], [0, 1])
        with self.assertRaisesRegex(RuntimeError, "original multi-material leaf ID missing"):
            bind_original_material_ids([original], 0, "Unknown", [
                {"id": 0, "triangles": 1, "name": "Base"},
                {"id": 10, "triangles": 1, "name": "Other"},
            ])

    def test_single_implicit_material_does_not_invent_other_ids(self):
        original = Mesh("single", [Material("Default Material")], [0, 0])
        records = [{"id": 0, "triangles": 2, "name": "Source material"}]
        bind_original_material_ids([original], 0, "Single", records)
        verify_original_material_ids([original], [records])

    def test_rejects_bad_source_contract(self):
        with self.assertRaisesRegex(RuntimeError, "duplicate original material ID"):
            parse_material_records([
                {"id": 0, "triangles": 1, "name": "one"},
                {"id": 0, "triangles": 1, "name": "two"},
            ])
        with self.assertRaisesRegex(RuntimeError, "invalid original material ID"):
            parse_material_records([{"id": 255, "triangles": 1, "name": "NoDraw"}])


if __name__ == "__main__":
    unittest.main()
