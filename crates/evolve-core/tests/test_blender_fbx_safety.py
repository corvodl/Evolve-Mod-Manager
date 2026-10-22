"""Dependency-free regression tests for binary FBX postprocessing."""
import pathlib
import struct
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "src"))
from blender_fbx_safety import (
    FBXValidationError, Node, _read_nodes, _write_nodes,
    _validate_geometry, optimize_binary_fbx,
)


def string(value):
    raw = value.encode()
    return b"S" + struct.pack("<I", len(raw)) + raw


def integer(value):
    return b"L" + struct.pack("<q", value)


def array(kind, values):
    fmt = {"i": "i", "d": "d"}[kind]
    raw = struct.pack("<" + str(len(values)) + fmt, *values)
    return kind.encode() + struct.pack("<III", len(values), 0, len(raw)) + raw


def node(name, props=b"", count=0, children=()):
    return Node(name.encode(), props, count, children, bool(children))


def obj(name, id_, subtype, children=()):
    return node(name, integer(id_) + string(name) + string(subtype), 3, children)


def sample(extra_objects=(), bad_uv=False):
    uv_values = [0., 0., 1., 0., 0., 1.]
    geom = obj("Geometry", 1, "Mesh", [
        node("Vertices", array("d", [0.,0.,0., 1.,0.,0., 0.,1.,0.]), 1),
        node("LayerElementUV", children=[
            node("UV", array("d", uv_values), 1),
            node("UVIndex", array("i", [0,1,99 if bad_uv else 2]), 1),
        ])
    ])
    skin = obj("Deformer", 2, "Skin")
    empty = obj("Deformer", 3, "Cluster", [
        node("Indexes", array("i", []), 1),
        node("Weights", array("d", []), 1),
    ])
    filled = obj("Deformer", 4, "Cluster", [
        node("Indexes", array("i", [0,1,2]), 1),
        node("Weights", array("d", [1.,1.,1.]), 1),
    ])
    matrix = [1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.]
    def pose(id_):
        return obj("Pose", id_, "BindPose", [
            node("Type", string("BindPose"), 1),
            node("NbPoseNodes", b"I"+struct.pack("<i", 1), 1),
            node("PoseNode", children=[
                node("Node", integer(7), 1),
                node("Matrix", array("d", matrix), 1),
            ]),
        ])
    def conn(a,b):
        return node("C", string("OO")+integer(a)+integer(b), 3)
    roots = [
        node("Objects", children=[geom,skin,empty,filled,pose(10),pose(11), *extra_objects]),
        node("Connections", children=[conn(3,2), conn(4,2), conn(2,1)]),
    ]
    return _write_nodes(7500, roots, bytes(25)+b"sample-trailer")


class FBXSafetyTests(unittest.TestCase):
    def test_sparse_cluster_and_identical_pose_optimization(self):
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder)/"test.fbx"
            path.write_bytes(sample())
            before = _read_nodes(path.read_bytes())[1][0]
            before_geo = before.children_named(b"Geometry")[0]
            stats = optimize_binary_fbx(path, embed_textures=False)
            self.assertEqual(stats["removed_empty_clusters"],1)
            self.assertEqual(stats["merged_bind_poses"],1)
            after = _read_nodes(path.read_bytes())[1][0]
            self.assertEqual(len(after.children_named(b"Pose")),1)
            self.assertEqual(after.children_named(b"Geometry")[0].props, before_geo.props)
            self.assertEqual(after.children_named(b"Geometry")[0].children[0].props,
                             before_geo.children[0].props)
            self.assertEqual(_validate_geometry(after),1)

    def test_rejects_invalid_uv_index_without_touching_file(self):
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder)/"bad.fbx"
            original = sample(bad_uv=True)
            path.write_bytes(original)
            with self.assertRaisesRegex(FBXValidationError, "UV indices"):
                optimize_binary_fbx(path)
            self.assertEqual(path.read_bytes(),original)

    def test_rejects_texture_refs_when_untextured(self):
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder)/"pink.fbx"
            path.write_bytes(sample(extra_objects=(obj("Texture",12,"TextureVideoClip"),)))
            with self.assertRaisesRegex(FBXValidationError,"references images"):
                optimize_binary_fbx(path,embed_textures=False)


if __name__ == "__main__":
    unittest.main()
