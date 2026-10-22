"""Bundled Blender 4.2+ external FBX export script for verified GLB intermediates.
Does not import arbitrary Python code from game assets.
"""
import bpy
import json
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
from blender_uv0 import prepare_original_uv
from blender_fbx_safety import (prepare_export_materials, validate_blender_meshes,
                               optimize_binary_fbx)

def run():
    # blender.exe --background --factory-startup --python <script> -- input.glb output.fbx 0|1
    argv = sys.argv[sys.argv.index("--") + 1:]
    if len(argv) != 4 or argv[2] not in ("0", "1") or argv[3] not in ("0", "1"):
        raise RuntimeError("Expected GLB, FBX, animation and texture embedding flags")
    source = Path(argv[0]).resolve(strict=True)
    destination = Path(argv[1]).absolute()
    include_animation = argv[2] == "1"
    embed_textures = argv[3] == "1"
    if source.suffix.casefold() != ".glb" or destination.suffix.casefold() != ".fbx":
        raise RuntimeError("GLB input and FBX output required")
    if destination.exists() or source == destination:
        raise RuntimeError("Refusing existing or overlapping FBX output")
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete(use_global=False)
    bpy.ops.import_scene.gltf(filepath=str(source))
    visible = [o for o in bpy.data.objects if o.type in ('MESH','ARMATURE')]
    meshes = prepare_original_uv([o for o in visible if o.type == 'MESH'])
    visible = [o for o in bpy.data.objects if o in visible or o in meshes]
    if not meshes:
        raise RuntimeError("Imported GLB contains no actual mesh")
    if not include_animation:
        # No embedded Actions or baked animation curves in default FBX export.
        for obj in bpy.data.objects:
            obj.animation_data_clear()
            if obj.data and hasattr(obj.data, 'animation_data_clear'):
                obj.data.animation_data_clear()
        for action in list(bpy.data.actions):
            bpy.data.actions.remove(action)
    # Preserve UV source names and ordering; never invent or reconstruct UV0.
    for obj in meshes:
        if obj.data.uv_layers:
            obj.data.uv_layers.active_index = 0
    prepare_export_materials(meshes, embed_textures)
    validate_blender_meshes(meshes, rigged=any(o.type == 'ARMATURE' for o in visible),
                            embed_textures=embed_textures)
    bpy.ops.object.select_all(action='DESELECT')
    for obj in visible:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    bpy.ops.export_scene.fbx(
        filepath=str(destination),
        use_selection=True,
        object_types={'MESH', 'ARMATURE', 'EMPTY'},
        apply_unit_scale=True,
        bake_space_transform=False,
        axis_forward='-Y', axis_up='Z',
        use_mesh_modifiers=True,
        mesh_smooth_type='FACE',
        use_armature_deform_only=False,
        add_leaf_bones=False,
        bake_anim=include_animation,
        bake_anim_use_nla_strips=include_animation,
        bake_anim_use_all_actions=include_animation,
        bake_anim_step=1.0,
        bake_anim_simplify_factor=0.0,
        path_mode='COPY',
        embed_textures=embed_textures
    )
    if not destination.is_file() or destination.stat().st_size < 256:
        raise RuntimeError("Blender did not create a valid FBX")
    optimized = optimize_binary_fbx(destination, embed_textures=embed_textures)
    print(json.dumps({
        "fbx_optimization": optimized,
        "meshes": len(meshes),
        "armatures": sum(o.type == 'ARMATURE' for o in visible),
        "animations_included": include_animation,
        "output_bytes": destination.stat().st_size
    }))

if __name__ == "__main__":
    run()
