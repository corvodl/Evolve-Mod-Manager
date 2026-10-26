"""Dependency-free regression tests for binary FBX postprocessing."""
import hashlib
import math
import zlib
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


def shaded_sample(normals=None, tangents=None, binormals=None, compressed=False):
    """Synthetic Blender FBX mesh with intact UV and explicitly stored TBN."""
    default_n = [0., 0., 1.] * 3
    default_t = [1., 0., 0.] * 3
    default_b = [0., 1., 0.] * 3
    values = (normals or default_n, tangents or default_t, binormals or default_b)
    version, roots, footer = _read_nodes(sample())
    geometry = roots[0].children_named(b"Geometry")[0]

    def vectors(name, payload, data):
        encoded = array("d", data)
        if compressed:
            packed = encoded[13:]
            zipped = zlib.compress(packed)
            encoded = b"d" + struct.pack("<III", len(data), 1, len(zipped)) + zipped
        geometry.children.append(node(name, children=[node(payload, encoded, 1)]))

    for (name, key), vec in zip((
        ("LayerElementNormal", "Normals"),
        ("LayerElementTangent", "Tangents"),
        ("LayerElementBinormal", "Binormals"),
    ), values):
        vectors(name, key, vec)
    return _write_nodes(version, roots, footer)


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

    def test_zero_uv_derivative_repairs_tangent_and_binormal_without_rewriting_mesh(self):
        # Emet-like fragments can have zero exported tangent-space vectors
        # on an otherwise valid triangulated mesh. Never discard the fragment.
        normals = [0., 0., 1.] * 3
        tangent = [0., 0., 0., 1., 0., 0., 1., 0., 0.]
        binormal = [0., 0., 0., 0., 1., 0., 0., 1., 0.]
        for compressed in (False, True):
            with self.subTest(compressed=compressed), tempfile.TemporaryDirectory() as folder:
                path = pathlib.Path(folder) / "emet_3p.fbx"
                path.write_bytes(shaded_sample(normals, tangent, binormal, compressed))
                before = _read_nodes(path.read_bytes())[1][0].children_named(b"Geometry")[0]
                original_normals = before.children_named(b"LayerElementNormal")[0].children[0].props
                original_uv = before.children_named(b"LayerElementUV")[0].children[0].props
                original_vertices = before.get(b"Vertices").props
                stats = optimize_binary_fbx(path, require_tangent_space=True)
                self.assertEqual(stats["repaired_tangent_vectors"], 2)
                after = _read_nodes(path.read_bytes())[1][0].children_named(b"Geometry")[0]
                self.assertEqual(after.children_named(b"LayerElementNormal")[0].children[0].props,
                                 original_normals)
                self.assertEqual(after.children_named(b"LayerElementUV")[0].children[0].props,
                                 original_uv)
                self.assertEqual(after.get(b"Vertices").props, original_vertices)
                _validate_geometry(_read_nodes(path.read_bytes())[1][0],
                                   require_tangent_space=True)

    def test_repairs_only_missing_binormal_when_original_tangent_is_valid(self):
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder) / "rigged.fbx"
            path.write_bytes(shaded_sample(
                tangents=[1., 0., 0.] * 3,
                binormals=[0., 0., 0.] + [0., 1., 0.] * 2,
            ))
            before = _read_nodes(path.read_bytes())[1][0].children_named(b"Geometry")[0]
            old_tangent = before.children_named(b"LayerElementTangent")[0].children[0].props
            stats = optimize_binary_fbx(path, require_tangent_space=True)
            self.assertEqual(stats["repaired_tangent_vectors"], 1)
            after = _read_nodes(path.read_bytes())[1][0].children_named(b"Geometry")[0]
            self.assertEqual(after.children_named(b"LayerElementTangent")[0].children[0].props,
                             old_tangent)
            _validate_geometry(_read_nodes(path.read_bytes())[1][0],
                               require_tangent_space=True)

    def test_invalid_source_normals_still_fail_without_modifying_fbx(self):
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder) / "invalid_normals.fbx"
            broken = shaded_sample(
                normals=[0., 0., 0.] + [0., 0., 1.] * 2,
                tangents=[0., 0., 0.] + [1., 0., 0.] * 2,
                binormals=[0., 0., 0.] + [0., 1., 0.] * 2,
            )
            path.write_bytes(broken)
            with self.assertRaisesRegex(FBXValidationError, "degenerate normal/tangent"):
                optimize_binary_fbx(path, require_tangent_space=True)
            self.assertEqual(path.read_bytes(), broken)

    def test_near_zero_nonzero_tangent_uses_valid_perpendicular_binormal(self):
        # Blender's UV-derivative residual can be nonzero even when no
        # meaningful unit tangent exists. Recover only that undefined corner.
        for compressed in (False, True):
            with self.subTest(compressed=compressed), tempfile.TemporaryDirectory() as folder:
                path = pathlib.Path(folder) / "emet_3p.fbx"
                path.write_bytes(shaded_sample(
                    tangents=[.0001, 0., 0.] + [1., 0., 0.] * 2,
                    compressed=compressed,
                ))
                original = _read_nodes(path.read_bytes())[1][0].children_named(b"Geometry")[0]
                normal_bytes = original.children_named(b"LayerElementNormal")[0].children[0].props
                binormal_bytes = original.children_named(b"LayerElementBinormal")[0].children[0].props
                original_uv = original.children_named(b"LayerElementUV")[0].children[0].props
                result = optimize_binary_fbx(path, require_tangent_space=True)
                self.assertEqual(result["repaired_tangent_vectors"], 1)
                geometry = _read_nodes(path.read_bytes())[1][0].children_named(b"Geometry")[0]
                self.assertEqual(geometry.children_named(b"LayerElementNormal")[0].children[0].props, normal_bytes)
                self.assertEqual(geometry.children_named(b"LayerElementBinormal")[0].children[0].props, binormal_bytes)
                self.assertEqual(geometry.children_named(b"LayerElementUV")[0].children[0].props, original_uv)
                _validate_geometry(_read_nodes(path.read_bytes())[1][0], require_tangent_space=True)

    def test_ambiguous_nonzero_tangent_and_parallel_partner_fail_closed(self):
        for tangent, binormal in (
            ([.02, 0., 0.] + [1., 0., 0.] * 2, None),
            ([.0001, 0., 0.] + [1., 0., 0.] * 2,
             [0., 0., 1.] + [0., 1., 0.] * 2),
        ):
            with self.subTest(tangent=tangent[0]), tempfile.TemporaryDirectory() as folder:
                path = pathlib.Path(folder) / "invalid_tangent.fbx"
                original = shaded_sample(tangents=tangent, binormals=binormal)
                path.write_bytes(original)
                with self.assertRaisesRegex(FBXValidationError, "degenerate normal/tangent"):
                    optimize_binary_fbx(path, require_tangent_space=True)
                self.assertEqual(path.read_bytes(), original)

    def test_rocky_omitted_zero_influence_cluster_arrays(self):
        # Blender can omit Indexes and Weights entirely for unused joints.
        # Those deformers must be removed without stripping weighted ones.
        variants = (
            [],
            [node("Indexes", array("i", []), 1)],
            [node("Weights", array("d", []), 1)],
        )
        for children in variants:
            with self.subTest(children=len(children)), tempfile.TemporaryDirectory() as folder:
                path = pathlib.Path(folder) / "rocky3.fbx"
                # Replace the ordinary empty cluster in a real binary-FBX
                # serialization while retaining the populated cluster.
                version, roots, footer = _read_nodes(sample())
                objects = roots[0]
                cluster = next(o for o in objects.children_named(b"Deformer")
                               if _properties(o)[0] == 3)
                cluster.children = children
                cluster.terminator = bool(children)
                path.write_bytes(_write_nodes(version, roots, footer))
                stats = optimize_binary_fbx(path)
                self.assertEqual(stats["weighted_clusters"], 1)
                self.assertEqual(stats["removed_empty_clusters"], 1)
                survivors = _read_nodes(path.read_bytes())[1][0].children_named(b"Deformer")
                self.assertEqual({ _properties(o)[0] for o in survivors }, {2, 4})

    def test_rocky_partial_weight_data_is_not_silently_discarded(self):
        variants = (
            [node("Indexes", array("i", [0]), 1)],
            [node("Weights", array("d", [1.0]), 1)],
        )
        for children in variants:
            with self.subTest(children=children[0].name), tempfile.TemporaryDirectory() as folder:
                path = pathlib.Path(folder) / "rocky3.fbx"
                version, roots, footer = _read_nodes(sample())
                cluster = next(o for o in roots[0].children_named(b"Deformer")
                               if _properties(o)[0] == 3)
                cluster.children = children
                cluster.terminator = True
                original = _write_nodes(version, roots, footer)
                path.write_bytes(original)
                with self.assertRaisesRegex(FBXValidationError, "missing indices/weights"):
                    optimize_binary_fbx(path)
                self.assertEqual(path.read_bytes(), original)

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

    def test_duplicate_fbx_video_records_share_one_embedded_normal(self):
        # Blender frequently exports one Video per material, even when the
        # materials reference the same original source normal image.
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            path = root / "emet.fbx"
            (root / "textures").mkdir()
            png = b"\x89PNG\r\n\x1a\n" + b"same-authored-normal-for-two-materials" * 4
            name = "emet_normal_" + hashlib.sha256(png).hexdigest()[:12] + ".png"
            (root / "textures" / name).write_bytes(png)
            image_nodes = (
                obj("Texture", 12, "TextureVideoClip", [
                    node("FileName", string(name), 1),
                ]),
                obj("Video", 13, "Clip", [
                    node("Filename", string("C:/export/" + name), 1),
                ]),
                obj("Texture", 14, "TextureVideoClip", [
                    node("FileName", string(name), 1),
                ]),
                obj("Video", 15, "Clip", [
                    node("Filename", string("textures/" + name), 1),
                ]),
            )
            path.write_bytes(sample(extra_objects=image_nodes))
            stats = optimize_binary_fbx(path, embed_textures=False)
            self.assertEqual(stats["embedded_normal_maps"], 1)
            objects = _read_nodes(path.read_bytes())[1][0]
            videos = objects.children_named(b"Video")
            self.assertEqual(len(videos), 2)
            for video in videos:
                self.assertEqual(_properties(video.get(b"Content"))[0], png)
                self.assertEqual(_properties(video.get(b"Filename"))[0],
                                 name.encode("utf-8"))

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
