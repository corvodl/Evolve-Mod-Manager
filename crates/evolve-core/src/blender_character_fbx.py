"""Merge individually verified rigged GLBs into one armature-bound FBX.
Only exact duplicate rest skeletons may be combined. No inferred weights or UVs.
"""
import bpy
import json
import sys
from pathlib import Path

def check(condition, reason):
    if not condition:
        raise RuntimeError("Character assembly: " + reason)

def same_matrix(a, b, tolerance=0.0002):
    return all(abs(a[i][j] - b[i][j]) <= tolerance
               for i in range(4) for j in range(4))

def same_skeleton(first, second):
    a = {b.name: b for b in first.data.bones}
    c = {b.name: b for b in second.data.bones}
    check(a.keys() == c.keys() and bool(a), "source bone names differ")
    check(same_matrix(first.matrix_world, second.matrix_world), "armature basis differs")
    for name, bone in a.items():
        other = c[name]
        check((bone.parent.name if bone.parent else "") ==
              (other.parent.name if other.parent else ""), "bone parent differs: " + name)
        check(same_matrix(bone.matrix_local, other.matrix_local), "bind pose differs: " + name)

def verify_mesh(mesh, armature):
    check(bool(mesh.data.vertices) and bool(mesh.data.polygons), "missing original mesh")
    check(bool(mesh.data.uv_layers), "missing original UV0")
    check(bool(mesh.vertex_groups) and any(v.groups for v in mesh.data.vertices),
          "missing original vertex weights")
    modifiers = [m for m in mesh.modifiers if m.type == "ARMATURE"]
    check(len(modifiers) == 1 and modifiers[0].object == armature,
          "incorrect source armature binding")
    return modifiers[0]

def main():
    argv = sys.argv[sys.argv.index("--") + 1:]
    check(len(argv) == 2, "expected parts manifest and destination")
    manifest_path = Path(argv[0]).resolve(strict=True)
    destination = Path(argv[1]).absolute()
    check(not destination.exists() and destination.suffix.lower() == ".fbx",
          "expected NEW .fbx output")
    check(manifest_path.stat().st_size <= 256 * 1024, "manifest too large")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    parts = manifest.get("parts")
    animate = manifest.get("include_animation")
    check(isinstance(parts, list) and 1 <= len(parts) <= 128 and type(animate) is bool,
          "invalid or excessive verified parts")

    bpy.ops.wm.read_factory_settings(use_empty=True)
    master = None
    meshes = []
    for part in parts:
        name, file = part.get("name"), part.get("source")
        check(isinstance(name, str) and 0 < len(name) <= 90 and
              isinstance(file, str), "invalid part description")
        source = Path(file).resolve(strict=True)
        check(source.suffix.lower() == ".glb" and source.parent == manifest_path.parent,
              "part must be a staged GLB")
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=str(source))
        added = set(bpy.data.objects) - before
        arms = [o for o in added if o.type == "ARMATURE"]
        imported = [o for o in added if o.type == "MESH"]
        check(len(arms) == 1 and len(imported) == 1, "part must have one mesh and skeleton")
        arm, mesh = arms[0], imported[0]
        modifier = verify_mesh(mesh, arm)
        if master is None:
            master = arm
        else:
            same_skeleton(master, arm)
            matrix = mesh.matrix_world.copy()
            modifier.object = master
            if mesh.parent == arm:
                mesh.parent = master
                mesh.matrix_world = matrix
            bpy.data.objects.remove(arm, do_unlink=True)
        mesh.name = name
        meshes.append(mesh)

    check(master is not None and len(meshes) == len(parts), "incomplete assembly")
    bone_count = len(master.data.bones)
    if not animate:
        for obj in bpy.data.objects:
            obj.animation_data_clear()
        for action in list(bpy.data.actions):
            bpy.data.actions.remove(action)
    bpy.ops.object.select_all(action="DESELECT")
    master.select_set(True)
    for mesh in meshes:
        mesh.select_set(True)
        verify_mesh(mesh, master)
    bpy.context.view_layer.objects.active = master
    bpy.ops.export_scene.fbx(
        filepath=str(destination), use_selection=True,
        object_types={"ARMATURE", "MESH"}, add_leaf_bones=False,
        use_armature_deform_only=False, axis_forward="-Z", axis_up="Y",
        bake_anim=animate, bake_anim_use_all_actions=animate,
        bake_anim_use_nla_strips=animate, bake_anim_simplify_factor=0,
        path_mode="COPY", embed_textures=True)
    check(destination.is_file() and destination.stat().st_size >= 256,
          "FBX export failed")
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.fbx(filepath=str(destination))
    result_arms = [o for o in bpy.data.objects if o.type == "ARMATURE"]
    result_meshes = [o for o in bpy.data.objects if o.type == "MESH"]
    check(len(result_arms) == 1 and len(result_arms[0].data.bones) == bone_count,
          "round-trip lost original skeleton")
    check(len(result_meshes) >= len(parts), "round-trip lost character parts")
    if animate:
        check(bool(bpy.data.actions), "FBX round-trip lost the verified CAF Action")
    for mesh in result_meshes:
        verify_mesh(mesh, result_arms[0])
    print(json.dumps({"parts": len(parts), "bones": bone_count,
                      "meshes": len(result_meshes), "bytes": destination.stat().st_size}))

if __name__ == "__main__":
    main()
