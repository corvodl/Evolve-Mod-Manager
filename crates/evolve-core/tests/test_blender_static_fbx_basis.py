"""Regression coverage for unrigged, nonembedded CryEngine FBX axis correction."""
import ast
import contextlib
import io
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
    def test_blender52_round_trip_keeps_only_snapshot_counts_after_scene_reset(self):
        """Reproduces Blender 5.2 removed StructRNA without requiring Blender."""
        script = (pathlib.Path(__file__).resolve().parents[1] / "src" /
                  "blender_glb_fbx.py").read_text(encoding="utf-8")
        run = next(node for node in ast.parse(script).body
                   if isinstance(node, ast.FunctionDef) and node.name == "run")
        reset = next(node.lineno for node in ast.walk(run)
                     if isinstance(node, ast.Call)
                     and isinstance(node.func, ast.Attribute)
                     and node.func.attr == "read_factory_settings")
        saved_names = {"exported_mesh_count", "exported_armature_count"}
        assignments = [
            stmt for stmt in run.body
            if isinstance(stmt, ast.Assign)
            and any(isinstance(target, ast.Name) and target.id in saved_names
                    for target in stmt.targets)
        ]
        self.assertEqual(
            {target.id for stmt in assignments for target in stmt.targets},
            saved_names)
        self.assertTrue(all(stmt.lineno < reset for stmt in assignments))
        reported = next(stmt for stmt in reversed(run.body)
                        if isinstance(stmt, ast.Expr)
                        and isinstance(stmt.value, ast.Call)
                        and isinstance(stmt.value.func, ast.Name)
                        and stmt.value.func.id == "print")

        class BlenderObject:
            def __init__(self, name):
                self.name = name
                self.removed = False

            @property
            def type(self):
                if self.removed:
                    raise ReferenceError("StructRNA of type Object has been removed")
                return self.name

        source_mesh = BlenderObject("MESH")
        source_armature = BlenderObject("ARMATURE")
        with tempfile.TemporaryDirectory() as folder:
            destination = pathlib.Path(folder) / "test.fbx"
            destination.write_bytes(b"FBX data")
            context = {"meshes": [source_mesh],
                       "visible": [source_mesh, source_armature],
                       "destination": destination,
                       "json": json, "optimized": {"embedded_normal_maps": 1},
                       "include_animation": False,
                       "root_name": "Emet", "combine_meshes": False}
            compile_snippet = lambda statements: compile(
                ast.fix_missing_locations(ast.Module(
                    body=statements, type_ignores=[])), "<lifecycle-test>", "exec")
            exec(compile_snippet(assignments), context)
            # bpy.ops.wm.read_factory_settings() destroys source objects.
            source_mesh.removed = True
            source_armature.removed = True
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                exec(compile_snippet([reported]), context)
            result = json.loads(output.getvalue())
            self.assertEqual(result["meshes"], 1)
            self.assertEqual(result["armatures"], 1)
            self.assertEqual(result["output_bytes"], destination.stat().st_size)

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
        displaced = Mesh([[-1., -2., 0.], [1., 2., 10.], [0., 0., 5.]],
                         offset=[0., 0., 4.])
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
