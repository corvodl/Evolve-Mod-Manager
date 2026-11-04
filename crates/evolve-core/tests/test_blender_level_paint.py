"""Exact categorical terrain LayerId export without Blender in the test runner."""
import json
import pathlib
import struct
import sys
import tempfile
import unittest
import zlib

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "src"))
from blender_level_paint import (
    TerrainPaintExportError, extract_original_layer_id_png,
    layer_id_sidecar_path, publish_layer_id_sidecar,
)


def png_chunk(kind, data):
    return (struct.pack(">I", len(data)) + kind + data +
            struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff))


def paint_png():
    ihdr = struct.pack(">IIBBBBB", 2, 1, 8, 6, 0, 0, 0)
    rgba = b"\x00\x03\x00\x00\xff\x07\x00\x00\xff"
    return (b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", ihdr) +
            png_chunk(b"IDAT", zlib.compress(rgba)) +
            png_chunk(b"IEND", b""))


def make_glb(png=None, marker=True):
    png = paint_png() if png is None else png
    doc = {
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": []}],
        "nodes": [],
        "buffers": [{"byteLength": len(png)}],
        "images": [{"name": "Original CryEngine LayerId (categorical)",
                    "mimeType": "image/png", "bufferView": 0}],
        "textures": [{"source": 0}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0,
                         "byteLength": len(png)}],
        "extras": {"source": "CryEngine Stage 2 level"}
    }
    if marker:
        doc["extras"]["original_categorical_terrain_paint"] = {
            "texture": 0, "width": 2, "height": 1,
            "layer_ids_not_surface_type_ids": True,
        }
    raw = json.dumps(doc).encode("utf-8")
    raw += b" " * (-len(raw) % 4)
    packed = png + b"\x00" * (-len(png) % 4)
    total = 12 + 8 + len(raw) + 8 + len(packed)
    return (b"glTF" + struct.pack("<II", 2, total) +
            struct.pack("<I", len(raw)) + b"JSON" + raw +
            struct.pack("<I", len(packed)) + b"BIN\x00" + packed)


class ExactTerrainLayerTests(unittest.TestCase):
    def test_embedded_layer_ids_are_extracted_without_resampling(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "scene.glb"
            path.write_bytes(make_glb())
            result = extract_original_layer_id_png(path)
            self.assertEqual(result, paint_png())
            fbx = pathlib.Path(tmp) / "scene.fbx"
            sidecar = publish_layer_id_sidecar(fbx, result)
            self.assertEqual(sidecar, layer_id_sidecar_path(fbx))
            self.assertEqual(sidecar.read_bytes(), paint_png())
            self.assertEqual(publish_layer_id_sidecar(fbx, result), sidecar)

    def test_scene_without_original_paint_creates_no_sidecar(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "scene.glb"
            path.write_bytes(make_glb(marker=False))
            self.assertIsNone(extract_original_layer_id_png(path))

    def test_mismatched_dimensions_or_corruption_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "invalid.glb"
            bad = bytearray(paint_png())
            bad[35] ^= 1  # mutate IDAT but keep original CRC
            path.write_bytes(make_glb(png=bytes(bad)))
            with self.assertRaises(TerrainPaintExportError):
                extract_original_layer_id_png(path)
            path.write_bytes(make_glb())
            source = bytearray(path.read_bytes())
            source[0] = ord("x")
            path.write_bytes(source)
            with self.assertRaises(TerrainPaintExportError):
                extract_original_layer_id_png(path)

    def test_existing_unrelated_paint_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = pathlib.Path(tmp) / "scene.fbx"
            sidecar = layer_id_sidecar_path(target)
            sidecar.write_bytes(b"user artwork")
            with self.assertRaises(TerrainPaintExportError):
                publish_layer_id_sidecar(target, paint_png())
            self.assertEqual(sidecar.read_bytes(), b"user artwork")


if __name__ == "__main__":
    unittest.main()
