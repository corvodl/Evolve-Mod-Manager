"""Pure-Python regression tests for source-normal and crease/UV fallback logic.

Run with python -m unittest discover -s scripts -p 'test_blender_mesh_shading.py'
No Blender installation required.
"""
import json
import sys
import tempfile
import types
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "crates" / "evolve-core" / "src"))

from blender_mesh_shading import glb_source_normals, sharp_edge_keys


def vec(x, y, z):
    return (x, y, z)


class _Poly:
    def __init__(self, loop_indices, normal, material_index=0):
        self.loop_indices = loop_indices
        self.normal = normal
        self.material_index = material_index


class Layers(list):
    def __init__(self, layer):
        super().__init__([layer])
        self.active = layer


def mesh(normals, uv_right=False, material_right=0):
    m = types.SimpleNamespace()
    m.polygons = [
        _Poly((0, 1, 2), normals[0]),
        _Poly((3, 4, 5), normals[1], material_right),
    ]
    m.loops = [
        types.SimpleNamespace(vertex_index=i)
        for i in (0, 1, 2, 2, 1, 3)
    ]
    coords = [(0., 0.), (1., 0.), (0., 1.),
              (.1 if uv_right else 0., 1.), (1., 0.), (1., 1.)]
    m.uv_layers = Layers(types.SimpleNamespace(data=[
        types.SimpleNamespace(uv=uv) for uv in coords
    ]))
    return m


def source_glb(with_normals=True):
    document = {
        "meshes": [{"primitives": [{
            "attributes": {"POSITION": 0, **({"NORMAL": 1} if with_normals else {})}
        }]}]
    }
    raw = json.dumps(document).encode("utf-8")
    return b"glTF" + b"\x02\x00\x00\x00" + b"\x00" * 4 + \
        len(raw).to_bytes(4, "little") + b"JSON" + raw


class ShadingTests(unittest.TestCase):
    def test_preserves_shading_mode_from_source(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "test.glb"
            path.write_bytes(source_glb(True))
            self.assertTrue(glb_source_normals(path))
            path.write_bytes(source_glb(False))
            self.assertFalse(glb_source_normals(path))

    def test_shallow_dihedral_allows_smoothing(self):
        m = mesh((vec(0, 0, 1), vec(0, .1, .995)))
        self.assertNotIn((1, 2), sharp_edge_keys(m))

    def test_hard_dihedral_is_sharp(self):
        m = mesh((vec(0, 0, 1), vec(0, 1, 0)))
        self.assertIn((1, 2), sharp_edge_keys(m))

    def test_uv_seam_is_sharp_even_for_parallel_faces(self):
        m = mesh((vec(0, 0, 1), vec(0, 0, 1)), uv_right=True)
        self.assertIn((1, 2), sharp_edge_keys(m))

    def test_material_boundary_is_sharp(self):
        m = mesh((vec(0, 0, 1), vec(0, 0, 1)), material_right=8)
        self.assertIn((1, 2), sharp_edge_keys(m))


if __name__ == "__main__":
    unittest.main()
