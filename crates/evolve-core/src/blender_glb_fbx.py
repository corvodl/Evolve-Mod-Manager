"""Bundled Blender 4.2+ external FBX export script for verified GLB intermediates.
Does not import arbitrary Python code from game assets.
"""
import bpy
import json
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
from blender_uv0 import prepare_original_uv
from blender_mesh_shading import (glb_source_normals, preserve_or_reconstruct_normals,
                                 normal_samples, verify_normal_samples)
from blender_material_ids import (material_records_from_glb, bind_original_material_ids,
                                  verify_original_material_ids, source_material_uv_distributions,
                                  verify_material_uv_distributions)
from blender_fbx_safety import (prepare_export_materials, validate_blender_meshes,
                               optimize_binary_fbx, normal_image_names)
from blender_static_fbx_basis import original_static_bounds, verify_static_world_bounds
from blender_export_hierarchy import (original_part_labels, name_static_meshes,
                                      character_root, combine_source_meshes)

def run():
    # Blender script flags: source.glb output.fbx animate embed combine
    argv = sys.argv[sys.argv.index("--") + 1:]
    if (len(argv) != 5 or any(flag not in ("0", "1") for flag in argv[2:])):
        raise RuntimeError("Expected GLB, FBX, animation, embedding and combine flags")
    source = Path(argv[0]).resolve(strict=True)
    destination = Path(argv[1]).absolute()
    include_animation = argv[2] == "1"
    embed_textures = argv[3] == "1"
    combine_meshes = argv[4] == "1"
    if source.suffix.casefold() != ".glb" or destination.suffix.casefold() != ".fbx":
        raise RuntimeError("GLB input and FBX output required")
    if destination.exists() or source == destination:
        raise RuntimeError("Refusing existing or overlapping FBX output")
    native_bounds = original_static_bounds(source)
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete(use_global=False)
    bpy.ops.import_scene.gltf(filepath=str(source))
    visible = [o for o in bpy.data.objects if o.type in ('MESH','ARMATURE','LIGHT')]
    source_lights = [o for o in visible if o.type == 'LIGHT']
    source_meshes = [o for o in visible if o.type == 'MESH']
    # A source level placement may contain several authored material leaves.
    # Blender's material splitting otherwise makes those pieces independent,
    # so group them by their ORIGINAL object before changing topology.
    with source.open('rb') as glb_file:
        header = glb_file.read(20)
        if header[:4] != b'glTF' or header[16:20] != b'JSON':
            raise RuntimeError('Invalid GLB scene before placement grouping')
        import struct
        json_len = struct.unpack_from('<I', header, 12)[0]
        if json_len > 64 * 1024 * 1024:
            raise RuntimeError('Oversized scene metadata')
        doc = json.loads(glb_file.read(json_len))
    level_scene = doc.get('extras', {}).get('source') == 'CryEngine Stage 2 level'
    # Multi-part original CGFs already have a parent placement node in glTF.
    # Preserve those real source hierarchies rather than regrouping each mesh.
    group_names = {node.get('name') for node in doc.get('nodes', [])
                   if node.get('children')
                   and node.get('extras', {}).get('source_id')}
    def source_group(obj):
        if obj.type != 'EMPTY' or not any(c.type == 'MESH' for c in obj.children):
            return False
        name = obj.name
        if name in group_names:
            return True
        stem, dot, suffix = name.rpartition('.')
        return bool(dot and suffix.isdigit() and stem in group_names)
    source_placement_groups = ([o for o in bpy.data.objects if source_group(o)]
                               if level_scene else [])
    placement_roots = []
    if level_scene:
        for index, mesh in enumerate(source_meshes):
            if mesh.parent in source_placement_groups:
                continue
            if len({p.material_index for p in mesh.data.polygons}) < 2:
                continue
            anchor = bpy.data.objects.new('EvolvePlacement_%06d' % index, None)
            bpy.context.scene.collection.objects.link(anchor)
            world = mesh.matrix_world.copy()
            anchor.matrix_world = world
            mesh.parent = anchor
            mesh.matrix_world = world
            placement_roots.append(anchor)
    source_material_ids = material_records_from_glb(source)
    if source_material_ids:
        bind_original_material_ids(source_meshes, 0, source.stem,
                                   source_material_ids)
    meshes = prepare_original_uv(source_meshes)
    if level_scene and placement_roots:
        for anchor in placement_roots:
            if not any(obj.parent == anchor for obj in meshes):
                raise RuntimeError('Original placement lost all material fragments')
    preserve_or_reconstruct_normals(meshes, glb_source_normals(source))
    visible = [o for o in bpy.data.objects if o in visible or o in meshes]
    if not meshes:
        raise RuntimeError("Imported GLB contains no actual mesh")
    if native_bounds is not None:
        if any(obj.type == 'ARMATURE' for obj in visible):
            raise RuntimeError("Static Z-up FBX geometry must not contain a rig")
        verify_static_world_bounds(meshes, native_bounds, "glTF import")
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
    if source_material_ids:
        verify_original_material_ids(meshes, [source_material_ids])
        source_material_uvs = source_material_uv_distributions(meshes)
    root_name = name_static_meshes(meshes, source.stem, original_part_labels(source))
    if combine_meshes:
        meshes, _ = combine_source_meshes(meshes, root_name=root_name)
        if source_material_ids:
            verify_original_material_ids(meshes, [source_material_ids])
            verify_material_uv_distributions(
                source_material_uvs, source_material_uv_distributions(meshes))
    source_shading = normal_samples(meshes)
    armatures = [obj for obj in visible if obj.type == "ARMATURE"]
    export_root = character_root(root_name, meshes, armatures)
    for anchor in placement_roots + source_placement_groups:
        world = anchor.matrix_world.copy()
        anchor.parent = export_root
        anchor.matrix_world = world
    # glTF punctual lights import as real Blender LIGHT objects.
    # Reparent them while retaining their world matrices, so every light is
    # independently movable when the resulting FBX is opened in Blender.
    for light in source_lights:
        world = light.matrix_world.copy()
        light.parent = export_root
        light.matrix_world = world
    visible = armatures + meshes + source_lights + placement_roots + source_placement_groups + [export_root]
    prepare_export_materials(meshes, embed_textures, destination)
    source_normal_maps = normal_image_names(meshes) if not embed_textures else set()
    validate_blender_meshes(meshes, rigged=any(o.type == 'ARMATURE' for o in visible),
                            embed_textures=embed_textures)
    bpy.ops.object.select_all(action='DESELECT')
    for obj in visible:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    bpy.ops.export_scene.fbx(
        filepath=str(destination),
        use_selection=True,
        use_custom_props=True,
        object_types={'MESH', 'ARMATURE', 'EMPTY', 'LIGHT'},
        apply_unit_scale=True,
        bake_space_transform=False,
        axis_forward='-Y', axis_up='Z',
        use_mesh_modifiers=True,
        mesh_smooth_type='EDGE',
        use_tspace=True,
        use_triangles=True,
        use_armature_deform_only=False,
        add_leaf_bones=False,
        bake_anim=include_animation,
        bake_anim_use_nla_strips=include_animation,
        bake_anim_use_all_actions=include_animation,
        bake_anim_step=1.0,
        bake_anim_simplify_factor=0.0,
        path_mode='COPY' if embed_textures else 'RELATIVE',
        embed_textures=embed_textures
    )
    if not destination.is_file() or destination.stat().st_size < 256:
        raise RuntimeError("Blender did not create a valid FBX")
    optimized = optimize_binary_fbx(destination, embed_textures=embed_textures,
                                    require_tangent_space=True)
    if not embed_textures:
        if optimized["embedded_normal_maps"] != len(source_normal_maps):
            raise RuntimeError("FBX lost one or more embedded normal maps")
    # The FBX round-trip removes all original bpy objects via factory reset.
    # Snapshot *primitive values* before destroying the source scene: Blender
    # 5.2 raises "StructRNA of type Object has been removed" even for o.type.
    # Never read attributes through source meshes/visible after this point.
    exported_mesh_count = len(meshes)
    exported_armature_count = sum(obj.type == 'ARMATURE' for obj in visible)
    exported_light_count = len(source_lights)
    exported_placement_groups = len(placement_roots)
    exported_source_groups = [o.name for o in source_placement_groups]
    if source_material_ids or native_bounds is not None:
        bpy.ops.wm.read_factory_settings(use_empty=True)
        bpy.ops.import_scene.fbx(filepath=str(destination))
        arms = [o for o in bpy.data.objects if o.type == 'ARMATURE']
        shapes = {bone.custom_shape for arm in arms for bone in arm.pose.bones
                  if bone.custom_shape is not None}
        roundtrip = [o for o in bpy.data.objects
                     if o.type == 'MESH' and o not in shapes]
        roundtrip_lights = [o for o in bpy.data.objects if o.type == 'LIGHT']
        if len(roundtrip_lights) != exported_light_count:
            raise RuntimeError('FBX light objects missing after Blender reimport')
        if level_scene:
            exported_groups = [o for o in bpy.data.objects
                               if o.name.startswith('EvolvePlacement_')]
            if len(exported_groups) != exported_placement_groups:
                raise RuntimeError('FBX lost editable multi-material placement groups')
            imported_group_names = {o.name for o in bpy.data.objects if o.type == 'EMPTY'}
            if any(name not in imported_group_names for name in exported_source_groups):
                raise RuntimeError('FBX lost one or more original multi-mesh placement groups')
        if native_bounds is not None:
            if arms:
                raise RuntimeError("Unrigged FBX unexpectedly gained an armature")
            verify_static_world_bounds(roundtrip, native_bounds, "FBX round-trip")
        verify_normal_samples(source_shading, normal_samples(roundtrip))
        if source_material_ids:
            verify_original_material_ids(roundtrip, [source_material_ids])
            verify_material_uv_distributions(
                source_material_uvs, source_material_uv_distributions(roundtrip))
    print(json.dumps({
        "fbx_optimization": optimized,
        "meshes": exported_mesh_count,
        "lights": exported_light_count,
        "editable_placement_groups": exported_placement_groups,
        "original_multi_mesh_groups": len(exported_source_groups),
        "armatures": exported_armature_count,
        "animations_included": include_animation,
        "character_root": root_name,
        "combined_meshes": combine_meshes,
        "output_bytes": destination.stat().st_size
    }))

if __name__ == "__main__":
    run()
