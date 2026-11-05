"""Unit tests for level FBX auditing without Blender or proprietary maps."""
import pathlib
import sys
import unittest

sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]/"src"))
from blender_level_roundtrip import (
    mesh_snapshot, light_snapshot, verify_mesh_snapshot,
    verify_light_snapshot, verify_independent_movement,
)


class Translation:
    def __init__(self, x, y, z):
        self.x, self.y, self.z = x, y, z
    def __iter__(self):
        return iter((self.x, self.y, self.z))


class Matrix:
    def __init__(self,x=0,y=0,z=0):
        self.translation=Translation(x,y,z)
    def __matmul__(self,vertex):
        return (vertex[0]+self.translation.x,
                vertex[1]+self.translation.y,
                vertex[2]+self.translation.z)
    def copy(self):
        return Matrix(*tuple(self.translation.__dict__.values()))


class Face:
    vertices=[0,1,2]


class Vertex:
    def __init__(self,xyz):
        self.co=xyz


class Object:
    def __init__(self,kind="MESH",origin=(0,0,0),uv=True):
        self.type=kind
        self.matrix_world=Matrix(*origin)
        self.data=type("Data",(),{})()
        if kind=="MESH":
            self.data.vertices=[Vertex(p) for p in
                ((0.,0.,0.),(1.,0.,0.),(0.,1.,0.))]
            self.data.polygons=[Face()]
            self.data.uv_layers=[1] if uv else []


class ViewLayer:
    def update(self):
        pass


class LevelRoundtripAuditTests(unittest.TestCase):
    def test_independent_placements_can_move_without_moving_sibling(self):
        objects=[Object(origin=(3,4,5)),Object(origin=(13,14,15))]
        before=mesh_snapshot(objects)
        self.assertEqual(verify_mesh_snapshot(before,mesh_snapshot(objects)),2)
        self.assertTrue(verify_independent_movement(objects,ViewLayer()))
        self.assertEqual(before,mesh_snapshot(objects))

    def test_detects_missing_or_fused_mesh(self):
        original=mesh_snapshot([Object(),Object(origin=(5,0,0))])
        with self.assertRaisesRegex(RuntimeError,"independent mesh objects changed"):
            verify_mesh_snapshot(original,mesh_snapshot([Object()]))

    def test_detects_displacement_and_triangle_loss(self):
        original=mesh_snapshot([Object()])
        with self.assertRaisesRegex(RuntimeError,"moved, rotated, or scaled"):
            verify_mesh_snapshot(original,mesh_snapshot([Object(origin=(0,0,2))]))
        missing=Object()
        missing.data.polygons=[Face(),Face()]
        with self.assertRaisesRegex(RuntimeError,"source triangle count changed"):
            verify_mesh_snapshot(original,mesh_snapshot([missing]))

    def test_preserves_original_uv0_without_guessing(self):
        with self.assertRaisesRegex(RuntimeError,"lost authored UV0"):
            verify_mesh_snapshot(mesh_snapshot([Object(uv=True)]),
                                 mesh_snapshot([Object(uv=False)]))

    def test_lights_are_individually_positioned(self):
        expected=light_snapshot([Object("LIGHT",(1,2,3)),Object("LIGHT",(5,6,7))])
        actual=light_snapshot([Object("LIGHT",(5,6,7)),Object("LIGHT",(1,2,3))])
        self.assertEqual(verify_light_snapshot(expected,actual),2)
        with self.assertRaisesRegex(RuntimeError,"position changed"):
            verify_light_snapshot(expected,
                light_snapshot([Object("LIGHT",(1,2,3)),Object("LIGHT",(5,6,9))]))

    def test_unsafe_nonfinite_world_positions_are_rejected(self):
        with self.assertRaisesRegex(RuntimeError,"nonfinite"):
            mesh_snapshot([Object(origin=(float("nan"),0,0))])


if __name__=="__main__":
    unittest.main()
