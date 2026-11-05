"""Synthetic compiled terrain fixtures. No original Evolve PAK bytes in CI."""
import struct
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from blender_compiled_terrain import (
    CompiledTerrainSourceError,
    compare_compiled_surface_sectors,
    original_heightmap_compiled_section,
    parse_compiled_terrain_surface_data,
)


def sample_compiled_terrain(*, last_nibble=15, palette=(3, 8)):
    # Matches observed v25 container and v7 LOD0 field sizes, but contains
    # entirely fabricated small scalar values, not any game-owned bytes.
    chunk = bytearray(struct.pack("<4BI4i2f",
                                  25, 0, 2, 1, 0,
                                  32, 1, 32, 1, 1 / 256, 0))
    node = bytearray(struct.pack("<hh6f2f2i", 7, 0, 0, 0, 0,
                                 32, 32, 1, 0, 1 / 256,
                                 33, len(palette)))
    words = [((i % 256) << 4) | (i % len(palette)) for i in range(33 * 33)]
    words[0] = last_nibble
    node.extend(struct.pack("<" + "H" * len(words), *words))
    node.extend(b"\xde\xde" + bytes(20) + bytes(palette))
    chunk.extend(node)
    chunk.extend(bytes(-len(chunk) % 4))
    struct.pack_into("<I", chunk, 4, len(chunk))
    return bytes(chunk)


class CompiledTerrainSourceTests(unittest.TestCase):
    def test_palette_index_is_categorical_and_reserved_nibbles_remain_unknown(self):
        terrain = parse_compiled_terrain_surface_data(sample_compiled_terrain())
        self.assertEqual(terrain.width, 32)
        self.assertEqual(terrain.sector_meters, 32)
        self.assertEqual(len(terrain.sectors), 1)
        vertex = terrain.sectors[(0, 0)]
        self.assertEqual(vertex.source_surface_type_at(0, 1), 8)
        self.assertIsNone(vertex.source_surface_type_at(0, 0))
        raster = terrain.source_surface_types()
        self.assertEqual(len(raster), 32 * 32)
        self.assertEqual(raster[0], 255)
        self.assertEqual(raster[1], 8)
        self.assertEqual(raster[32], 3)
        with self.assertRaises(IndexError):
            vertex.source_surface_type_at(33, 0)

    def test_comparison_requires_identical_packed_values_and_palette(self):
        original = sample_compiled_terrain()
        self.assertEqual(compare_compiled_surface_sectors(original, original), 1)
        with self.assertRaisesRegex(CompiledTerrainSourceError, "differ"):
            compare_compiled_surface_sectors(
                original, sample_compiled_terrain(last_nibble=0))
        with self.assertRaisesRegex(CompiledTerrainSourceError, "differ"):
            compare_compiled_surface_sectors(
                original, sample_compiled_terrain(palette=(3, 9)))

    def test_source_heightmap_tail_is_bounded_and_uncompressed(self):
        compiled = sample_compiled_terrain()
        marker = b"TerrainCompiledData"
        wrapped = (b"\xff" + bytes(50) + marker
                   + struct.pack("<III", len(compiled), len(compiled), 0)
                   + compiled)
        self.assertEqual(original_heightmap_compiled_section(wrapped), compiled)
        with self.assertRaises(CompiledTerrainSourceError):
            original_heightmap_compiled_section(wrapped[:-1])
        with self.assertRaises(CompiledTerrainSourceError):
            original_heightmap_compiled_section(wrapped.replace(marker, b"x" * len(marker)))

    def test_unknown_or_truncated_layout_fails_closed(self):
        good = sample_compiled_terrain()
        for bad in (b"", good[:4], good[:-20], bytes(len(good))):
            with self.assertRaises(CompiledTerrainSourceError):
                parse_compiled_terrain_surface_data(bad)
        corrupt = bytearray(good)
        corrupt[0] = 24  # Unsupported compilation version
        with self.assertRaises(CompiledTerrainSourceError):
            parse_compiled_terrain_surface_data(bytes(corrupt))


if __name__ == "__main__":
    unittest.main()
