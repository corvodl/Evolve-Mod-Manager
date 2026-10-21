"""Test the real Blender character compatibility function without importing bpy.

The 2012 Rocky FBX can have 47-bone and 48-bone verified source armatures.
Only identical skeletons should be unified; otherwise distinct native rigs
must be retained, never relabelled to appear compatible.
"""
import ast
from pathlib import Path
import unittest


def compatibility_functions():
    script = Path(__file__).resolve().parents[1] / "src" / "blender_character_fbx.py"
    parsed = ast.parse(script.read_text(encoding="utf-8"), filename=str(script))
    selected = [node for node in parsed.body
                if isinstance(node, ast.FunctionDef)
                and node.name in {"same_matrix", "same_skeleton"}]
    assert len(selected) == 2, "The actual character FBX compatibility check is missing"
    module = ast.Module(body=selected, type_ignores=[])
    names = {}
    exec(compile(ast.fix_missing_locations(module), str(script), "exec"), names)
    return names["same_skeleton"]


def identity():
    return [[1.0 if row == col else 0.0 for col in range(4)]
            for row in range(4)]


class Bone:
    def __init__(self, name, parent=None, matrix=None):
        self.name = name
        self.parent = parent
        self.matrix_local = matrix or identity()


class Rig:
    def __init__(self, names, changed_parent=None, changed_pose=None):
        self.matrix_world = identity()
        bones = []
        for index, name in enumerate(names):
            parent = bones[0] if index and changed_parent != index else None
            bone = Bone(name, parent=parent)
            if changed_pose == index:
                bone.matrix_local[0][3] = 5.0
            bones.append(bone)
        self.data = type("RigData", (), {"bones": bones})()


class RockySourceArmatureTests(unittest.TestCase):
    def test_2012_47_and_48_bones_are_kept_distinct(self):
        same_skeleton = compatibility_functions()
        bones47 = [f"rocky_bone_{i:02}" for i in range(47)]
        body = Rig(bones47)
        another_body = Rig(bones47)
        left_arm = Rig(bones47 + ["rocky_extra_joint"])
        self.assertTrue(same_skeleton(body, another_body))
        self.assertFalse(same_skeleton(body, left_arm))
        # The optional extra joint remains present, with its original name.
        self.assertEqual(len(left_arm.data.bones), 48)
        self.assertEqual(left_arm.data.bones[-1].name, "rocky_extra_joint")

    def test_equal_bone_counts_are_not_enough_for_retargeting(self):
        same_skeleton = compatibility_functions()
        base = [f"rocky_bone_{i:02}" for i in range(47)]
        reference = Rig(base)
        self.assertFalse(same_skeleton(reference, Rig(base, changed_parent=3)))
        self.assertFalse(same_skeleton(reference, Rig(base, changed_pose=3)))
        self.assertFalse(same_skeleton(reference, Rig(base[:-1] + ["other_bone"])))
        shifted = Rig(base)
        shifted.matrix_world[1][3] = 0.2
        self.assertFalse(same_skeleton(reference, shifted))

    def test_mixed_rig_export_is_explicitly_historical_only(self):
        code = (Path(__file__).resolve().parents[1] / "src" /
                "blender_character_fbx.py").read_text(encoding="utf-8")
        self.assertIn('manifest.get("allow_distinct_historical_armatures", False)', code)
        self.assertIn('not armatures or distinct_historical', code)
        self.assertIn('not (distinct_historical and animate)', code)
        self.assertIn('"FBX round-trip rebound original mesh', code)


if __name__ == "__main__":
    unittest.main()
