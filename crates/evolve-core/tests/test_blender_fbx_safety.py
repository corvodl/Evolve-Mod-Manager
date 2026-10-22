"""Dependency-free regression tests for binary FBX postprocessing."""
import hashlib
import pathlib
import struct
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "src"))
from blender_fbx_safety import (
    FBXValidationError, Node, _read_nodes, _write_nodes,
    _validate_geometry, optimize_binary_fbx, _properties,
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

    def test_original_normal_is_embedded_without_external_dependencies(self):
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            path = root / "emet.fbx"
            textures = root / "textures"
            textures.mkdir()
            png = b"\x89PNG\r\n\x1a\n" + b"synthetic-source-normal-map" * 5
            filename = "emet_normal_" + hashlib.sha256(png).hexdigest()[:12] + ".png"
            (textures / filename).write_bytes(png)
            temp_path = "C:/Temporary/Export/" + filename
            image_nodes = (
                obj("Texture", 12, "TextureVideoClip", [
                    node("FileName", string(temp_path), 1),
                    node("RelativeFilename", string(temp_path), 1),
                ]),
                obj("Video", 13, "Clip", [
                    node("Filename", string(temp_path), 1),
                    node("RelativeFilename", string(temp_path), 1),
                ]),
            )
            path.write_bytes(sample(extra_objects=image_nodes))
            stats = optimize_binary_fbx(path, embed_textures=False)
            self.assertEqual(stats["embedded_normal_maps"], 1)
            objects = _read_nodes(path.read_bytes())[1][0]
            videos = objects.children_named(b"Video")
            self.assertEqual(len(videos), 1)
            self.assertEqual(_properties(videos[0].get(b"Content"))[0], png)
            self.assertEqual(_properties(videos[0].get(b"Filename"))[0],
                             filename.encode("utf-8"))
            self.assertEqual(_properties(videos[0].get(b"RelativeFilename"))[0],
                             filename.encode("utf-8"))
            texture = objects.children_named(b"Texture")[0]
            self.assertEqual(_properties(texture.get(b"FileName"))[0],
                             filename.encode("utf-8"))
            # The PNG exists only in the exporter scratch directory. The
            # Rust publisher must never copy this folder to user output.
            self.assertTrue((textures / filename).exists())

    def test_missing_normal_map_or_diffuse_reference_fails_closed(self):
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder) / "emet.fbx"
            path.write_bytes(sample(extra_objects=(
                obj("Texture", 12, "TextureVideoClip", [
                    node("FileName", string("textures/body_diffuse.png"), 1),
                ]),
            )))
            before = path.read_bytes()
            with self.assertRaisesRegex(
                    FBXValidationError, "unexpected or missing texture"):
                optimize_binary_fbx(path, embed_textures=False)
            self.assertEqual(path.read_bytes(), before)

    def test_staged_normal_payload_must_match_filename_digest(self):
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            path = root / "emet.fbx"
            (root / "textures").mkdir()
            name = "emet_normal_123456789abc.png"
            (root / "textures" / name).write_bytes(
                b"\x89PNG\r\n\x1a\n" + b"wrong hash")
            path.write_bytes(sample(extra_objects=(
                obj("Texture", 12, "TextureVideoClip", [
                    node("FileName", string(name), 1),
                ]),
                obj("Video", 13, "Clip", [
                    node("Filename", string(name), 1),
                ]),
            )))
            before = path.read_bytes()
            with self.assertRaisesRegex(
                    FBXValidationError, "not its original PNG"):
                optimize_binary_fbx(path, embed_textures=False)
            self.assertEqual(path.read_bytes(), before)

    def test_no_normal_images_does_not_invent_embedded_media(self):
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder) / "untextured.fbx"
            path.write_bytes(sample())
            stats = optimize_binary_fbx(path, embed_textures=False)
            self.assertEqual(stats["embedded_normal_maps"], 0)
            objects = _read_nodes(path.read_bytes())[1][0]
            self.assertFalse(objects.children_named(b"Video"))
            self.assertFalse(objects.children_named(b"Texture"))


if __name__ == "__main__":
    unittest.main()
