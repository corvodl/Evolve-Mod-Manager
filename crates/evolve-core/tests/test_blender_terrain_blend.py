"""Synthetic scalar tests; no original game shader binaries checked into Git."""
import math
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from blender_terrain_blend import (
    TerrainBlendInputsMissing,
    get_layer_blending_value,
    bake_terrain_layer_color,
)


def recovered_formula(height, alpha, factor, falloff):
    # Independent literal transcription of parsed ShadeLib.cfib operation.
    sat = lambda x: max(0.0, min(1.0, x))
    blend_map = sat(sat(alpha * 2.0 - 1.0) + height)
    return sat((alpha * blend_map * (1.0 + factor)) ** falloff)


class EvolveTerrainBlendTests(unittest.TestCase):
    def test_source_backed_scalar_against_literal_transcription(self):
        for height in (0.0, 0.2, 0.5, 1.0):
            for alpha in (0.0, 0.25, 0.5, 0.75, 1.0):
                for factor, falloff in ((0, 1), (1, 2), (0.5, 0.5)):
                    with self.subTest(height=height, alpha=alpha,
                                      factor=factor, falloff=falloff):
                        self.assertAlmostEqual(
                            get_layer_blending_value(
                                height, alpha, factor, falloff,
                                has_source_height=True, has_source_alpha=True),
                            recovered_formula(height, alpha, factor, falloff))

    def test_zero_alpha_cannot_invent_a_layer(self):
        self.assertEqual(get_layer_blending_value(
            1, 0, 1, 2, has_source_height=True, has_source_alpha=True), 0)

    def test_requires_both_source_inputs(self):
        for kwargs in (
            {},
            {"has_source_height": True},
            {"has_source_alpha": True},
        ):
            with self.assertRaises(TerrainBlendInputsMissing):
                get_layer_blending_value(0.7, 0.75, 1, 2, **kwargs)

    def test_nonfinite_and_unverified_parameters_fail(self):
        for val in (math.nan, math.inf, -math.inf):
            with self.assertRaises(TerrainBlendInputsMissing):
                get_layer_blending_value(val, 0.5, 0.5, 1,
                    has_source_height=True, has_source_alpha=True)
        with self.assertRaises(TerrainBlendInputsMissing):
            get_layer_blending_value(0.5, 0.5, 0.5, 0,
                has_source_height=True, has_source_alpha=True)

    def test_bake_is_opt_in_and_preserves_source_macro_for_zero_weight(self):
        base = (0.1, 0.2, 0.3)
        with self.assertRaises(TerrainBlendInputsMissing):
            bake_terrain_layer_color(base, (0.9, 0.9, 0.9), 1,
                                     0.7, 0.8, 1, 2)
        self.assertEqual(bake_terrain_layer_color(
            base, (1, 1, 1), 1, 1, 0, 1, 2,
            has_source_height=True, has_source_alpha=True), base)


if __name__ == "__main__":
    unittest.main()
