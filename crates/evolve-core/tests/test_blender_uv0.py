"""Topology-aware UV0 reconstruction, without Blender or game assets."""
import unittest
import pathlib
import sys
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "src"))

from blender_uv0 import reconstruct_uv0


class OriginalUVRepairTests(unittest.TestCase):
    def test_snaps_tiny_cracks_only_across_matching_shared_edges(self):
        # Two faces share an edge through duplicated render vertices.
        faces = ((0,1,2),(3,4,5))
        loop_vertices = (0,1,2,3,4,5)
        positions = ((0,0),(1,0),(0,1),(1,0),(0,1),(1,1))
        coords = ((0.,0.),(.5,0.),(.5,.8),
                  (.5000002,0.),(.5000002,.8000001),(.9,.9))
        updated, info = reconstruct_uv0(
            faces,loop_vertices,coords,vertex_keys=positions)
        self.assertEqual(info['shared_edges_verified'],1)
        self.assertGreaterEqual(info['snapped_corners'],2)
        self.assertEqual(updated[1],updated[3])
        self.assertEqual(updated[2],updated[4])
        self.assertEqual(updated[0],coords[0])

    def test_connected_exact_original_uvs_are_byte_identical(self):
        # Three adjacent faces form a closed fan around a shared vertex.
        # Repeated cooked render loops must remain numerically identical.
        faces = ((0, 1, 2), (3, 4, 5), (6, 7, 8))
        keys = (0, 1, 2, 0, 2, 3, 0, 3, 1)
        center = (0.5364838242530823, 0.2590183615684509)
        around = ((0.8123141527175903, 0.1927318423986435),
                  (0.10628369450569153, 0.9662139415740967),
                  (0.6735265851020813, 0.1893846094608307))
        coords = (center, around[0], around[1],
                  center, around[1], around[2],
                  center, around[2], around[0])
        repaired, info = reconstruct_uv0(faces, keys, coords)
        self.assertEqual(repaired, coords)
        self.assertEqual(info['shared_edges_verified'], 3)
        self.assertEqual(info['snapped_corners'], 0)
        self.assertFalse(info['changed'])

    def test_most_common_original_uv_is_anchor_for_tiny_crack(self):
        faces = ((0, 1, 2), (3, 4, 5), (6, 7, 8))
        keys = (0, 1, 2, 0, 2, 3, 0, 3, 1)
        original = (0.31743690371513367, 0.7124837636947632)
        other = ((0.6735265851020813, 0.1893846094608307),
                 (0.10628369450569153, 0.9662139415740967),
                 (0.8123141527175903, 0.1927318423986435))
        slight_crack = (original[0] + 3e-7, original[1])
        coords = (original, other[0], other[1],
                  original, other[1], other[2],
                  slight_crack, other[2], other[0])
        repaired, info = reconstruct_uv0(faces, keys, coords)
        self.assertEqual(repaired[0], original)
        self.assertEqual(repaired[3], original)
        self.assertEqual(repaired[6], original)
        self.assertEqual(info['snapped_corners'], 1)
        self.assertEqual(info['shared_edges_verified'], 3)

    def test_preserves_mirrored_or_deliberately_split_island_border(self):
        faces = ((0,1,2),(3,4,5))
        keys = (0,1,2,1,2,3)
        coords = ((0.,0.),(.7,0.),(.7,.8),
                  (.2,0.),(.2,.8),(.9,.9))
        updated,info = reconstruct_uv0(faces,keys,coords)
        self.assertEqual(updated,coords)
        self.assertEqual(info['shared_edges_verified'],0)

    def test_repeat_tile_shift_only_when_sampler_verified(self):
        faces=((0,1,2),)
        coords=((1.2,2.1),(1.8,2.1),(1.5,2.7))
        old,_=reconstruct_uv0(faces,(0,1,2),coords,repeat_texture=False)
        new,info=reconstruct_uv0(faces,(0,1,2),coords,repeat_texture=True)
        self.assertEqual(old,coords)
        self.assertEqual(info['shifted_repeat_islands'],1)
        self.assertAlmostEqual(new[0][0],.2)
        self.assertAlmostEqual(new[0][1],.1)

    def test_does_not_shift_large_or_cross_tile_islands(self):
        faces=((0,1,2),)
        coords=((.8,0.),(1.2,.4),(.5,.9))
        repaired,info=reconstruct_uv0(
            faces,(0,1,2),coords,repeat_texture=True)
        self.assertEqual(repaired,coords)
        self.assertGreater(info['unshiftable_axes'],0)

    def test_rejects_unusable_uv_data(self):
        with self.assertRaisesRegex(ValueError, 'nonfinite'):
            reconstruct_uv0(((0,1,2),),(0,1,2),((0,0),(1,0),(float('nan'),1)))
        with self.assertRaisesRegex(ValueError, 'tolerance'):
            reconstruct_uv0(((0,1,2),),(0,1,2),((0,0),(1,0),(0,1)),
                            epsilon=.001)


if __name__ == '__main__':
    unittest.main()
