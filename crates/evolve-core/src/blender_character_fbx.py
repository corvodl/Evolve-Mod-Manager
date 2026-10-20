"""Merge individually verified rigged GLBs into one armature-bound FBX.
Only exact duplicate rest skeletons may be combined. No inferred weights or UVs.
"""
import bpy
import json
import math
import sys
import traceback
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

def referenced_bone_display_shapes(armatures):
    """Only hide shapes actually referenced by pose bones, never a genuine mesh
    merely named Icosphere (which some real Evolve parts may use).
    Blender 5.2 FBX import can create a display icosphere that is not character geometry.
    """
    return {bone.custom_shape for arm in armatures for bone in arm.pose.bones
            if bone.custom_shape is not None}

def actual_meshes(objects, armatures):
    custom_shapes = referenced_bone_display_shapes(armatures)
    return [obj for obj in objects
            if obj.type == "MESH" and obj not in custom_shapes]

def mesh_uv_bounds(mesh):
    uv = mesh.data.uv_layers[0].data
    check(len(uv) == len(mesh.data.loops) and bool(uv),
          "original UV0 loop data is missing")
    values = [[float(item.uv[axis]) for item in uv] for axis in range(2)]
    check(all(math.isfinite(value) for axis in values for value in axis),
          "nonfinite original UV0")
    return [(min(axis), max(axis)) for axis in values]

def append_uv_loops(mesh, coordinates):
    # FBX can reorder faces and split vertices at UV seams. Loop UV values,
    # unlike vertex order, still describe the authored per-corner coordinates.
    layer = mesh.data.uv_layers[0]
    for item in layer.data:
        coordinates[0].append(float(item.uv[0]))
        coordinates[1].append(float(item.uv[1]))

def verify_uv_loop_distributions(source, destination):
    # Compare the entire coordinate distribution on both axes rather than
    # only the bounding rectangle. This rejects missing/replaced loops and
    # large UV rearrangements while tolerating FBX's face reordering and
    # sub-millipixel float32 conversion drift.
    for axis in range(2):
        before, after = sorted(source[axis]), sorted(destination[axis])
        check(len(before) == len(after) and bool(before),
              "FBX round-trip changed original UV0 loop count")
        check(all(abs(a - b) <= 0.001 for a, b in zip(before, after)),
              "FBX round-trip changed original UV0 coordinates")

def verify_mesh(mesh, armature):
    check(bool(mesh.data.vertices) and bool(mesh.data.polygons), "missing original mesh")
    check(bool(mesh.data.uv_layers), "missing original UV0")
    modifiers = [m for m in mesh.modifiers if m.type == "ARMATURE"]
    check(len(modifiers) == 1 and modifiers[0].object == armature,
          "incorrect source armature binding")
    bones = {bone.name for bone in armature.data.bones}
    weighted = 0
    for vertex in mesh.data.vertices:
        valid = False
        for assignment in vertex.groups:
            check(assignment.group < len(mesh.vertex_groups),
                  "bone weight group index out of bounds")
            check(mesh.vertex_groups[assignment.group].name in bones,
                  "bone weight references a nonexistent source bone")
            check(math.isfinite(assignment.weight) and 0 <= assignment.weight <= 1.00001,
                  "invalid original vertex weight")
            valid = valid or assignment.weight > 0
        weighted += int(valid)
    check(weighted > 0, "missing original vertex weights")
    return modifiers[0], weighted, mesh_uv_bounds(mesh)

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
    source_uv_bounds = []
    source_uv_loops = [[], []]
    source_weighted_vertices = 0
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
        imported = actual_meshes(added, arms)
        check(len(arms) == 1 and len(imported) == 1, "part must have one mesh and skeleton")
        arm, mesh = arms[0], imported[0]
        modifier, weighted, uv_bounds = verify_mesh(mesh, arm)
        source_weighted_vertices += weighted
        source_uv_bounds.append(uv_bounds)
        append_uv_loops(mesh, source_uv_loops)
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
    source_hierarchy = {
        bone.name: (bone.parent.name if bone.parent else None)
        for bone in master.data.bones
    }
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
    result_meshes = actual_meshes(bpy.data.objects, result_arms)
    display_shapes = referenced_bone_display_shapes(result_arms)
    check(len(result_arms) == 1 and len(result_arms[0].data.bones) == bone_count,
          "round-trip lost original skeleton")
    target_hierarchy = {
        bone.name: (bone.parent.name if bone.parent else None)
        for bone in result_arms[0].data.bones
    }
    check(source_hierarchy == target_hierarchy,
          "FBX round-trip changed the original bone hierarchy")
    check(len(result_meshes) >= len(parts), "round-trip lost character parts")
    if animate:
        check(bool(bpy.data.actions), "FBX round-trip lost the verified CAF Action")
    imported_weighted_vertices = 0
    target_bounds = []
    target_uv_loops = [[], []]
    for mesh in result_meshes:
        _, weighted, bounds = verify_mesh(mesh, result_arms[0])
        imported_weighted_vertices += weighted
        target_bounds.append(bounds)
        append_uv_loops(mesh, target_uv_loops)
    check(imported_weighted_vertices > 0 and source_weighted_vertices > 0,
          "round-trip lost original bone weights")
    # FBX may split original vertices at UV seams. Validate the global authored
    # UV0 envelope, not vertex ordering or exact mesh counts.
    for axis in range(2):
        original_lo = min(bounds[axis][0] for bounds in source_uv_bounds)
        original_hi = max(bounds[axis][1] for bounds in source_uv_bounds)
        imported_lo = min(bounds[axis][0] for bounds in target_bounds)
        imported_hi = max(bounds[axis][1] for bounds in target_bounds)
        check(abs(original_lo - imported_lo) <= 0.002
              and abs(original_hi - imported_hi) <= 0.002,
              "FBX round-trip changed original UV0 bounds")
    verify_uv_loop_distributions(source_uv_loops, target_uv_loops)
    print(json.dumps({"parts": len(parts), "bones": bone_count,
                      "meshes": len(result_meshes), "display_shapes_excluded": len(display_shapes),
                      "weighted_vertices": imported_weighted_vertices,
                      "uv_loops": len(target_uv_loops[0]),
                      "bytes": destination.stat().st_size}))

if __name__ == "__main__":
    try:
        main()
    except Exception:
        traceback.print_exc()
        sys.exit(1)
