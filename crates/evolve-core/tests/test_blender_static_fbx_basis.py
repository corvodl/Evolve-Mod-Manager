"""Regression coverage for unrigged, nonembedded CryEngine FBX axis correction."""
import json
import math
import pathlib
import struct
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "src"))
from blender_static_fbx_basis import original_static_bounds, verify_static_world_bounds


class Matrix:
    def __init__(self, offset=(0, 0, 0)):
        self.offset = offset

    def __matmul__(self, point):
        return [a + b for a, b in zip(point, self.offset)]


class Vertex:
    def __init__(self, xyz):
        self.co = xyz


class Mesh:
    def __init__(self, vertices, offset=(0, 0, 0)):
        self.data = type("MeshData", (), {"vertices": [Vertex(v) for v in vertices]})()
        self.matrix_world = Matrix(offset)


def source_glb(*, basis=True, angle=None):
    s = math.sqrt(0.5)
    quaternion = [-s, 0, 0, s] if angle is None else angle
    root = (
        {"name": "CryEngine static source axis", "rotation": quaternion, "children": [0]}
        if basis else None
    )
    doc = {
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [1 if basis else 0]}],
        "nodes": [{"mesh": 0}] + ([root] if basis else []),
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
        "accessors": [{"type": "VEC3", "count": 3,
                       "min": [-1., -2., 0.],
                       "max": [1., 2., 10.]}],
        "extras": {"evolve_static_fbx_basis": "cryengine_z_up_to_gltf_y_up"} if basis else {},
    }
    raw = json.dumps(doc).encode()
    raw += b" " * ((-len(raw)) % 4)
    return (b"glTF" + struct.pack("<I", 2)
            + struct.pack("<I", len(raw) + 20)
            + struct.pack("<I", len(raw)) + b"JSON" + raw)


class StaticFBXBasisTests(unittest.TestCase):
    def test_native_source_coordinates_are_verified(self):
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder) / "Emet.glb"
            path.write_bytes(source_glb())
            expected = original_static_bounds(path)
            self.assertEqual(expected, ([-1., -2., 0.], [1., 2., 10.]))
            mesh = Mesh([[-1., -2., 0.], [1., 2., 10.], [0., 0., 5.]])
            self.assertTrue(verify_static_world_bounds(
                [mesh], expected, "FBX round-trip"))

    def test_fails_if_standing_z_up_model_is_tipped_onto_y_axis(self):
        original = ([-1., -2., 0.], [1., 2., 10.])
        sideways = Mesh([[-1., 0., -2.], [1., -10., 2.], [0., -5., 0.]])
        with self.assertRaisesRegex(RuntimeError, "rotated, scaled, or displaced"):
            verify_static_world_bounds([sideways], original, "glTF import")

    def test_fails_if_height_is_scaled_or_parent_transform_missing(self):
        original = ([-1., -2., 0.], [1., 2., 10.])
        displaced = Mesh([[-1., -2., 0.], [1., 2., 10.]], offset=[0., 0., 4.])
        with self.assertRaisesRegex(RuntimeError, "rotated, scaled, or displaced"):
            verify_static_world_bounds([displaced], original, "FBX round-trip")

    def test_no_extra_conversion_for_generic_level_intermediate(self):
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder) / "terrain.glb"
            path.write_bytes(source_glb(basis=False))
            self.assertIsNone(original_static_bounds(path))

    def test_refuses_wrong_basis_even_with_marker(self):
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder) / "wrong.glb"
            path.write_bytes(source_glb(angle=[0, 0, 0, 1]))
            with self.assertRaisesRegex(RuntimeError, "wrong CryEngine Z-up basis"):
                original_static_bounds(path)


if __name__ == "__main__":
    unittest.main()
