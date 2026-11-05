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
from blender_level_paint import (extract_original_layer_id_png,
                                 layer_id_sidecar_path, publish_layer_id_sidecar)
from blender_level_transition_edges import (
    source_transition_boundaries, transition_sidecar_path,
    publish_transition_sidecar
)
from blender_level_roundtrip import (mesh_snapshot, light_snapshot,
                                     verify_mesh_snapshot, verify_light_snapshot,
                                     verify_independent_movement)
from blender_export_hierarchy import (original_part_labels, name_static_meshes,
                                      character_root, combine_source_meshes, clean_name)

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
    original_paint_png = (extract_original_layer_id_png(source) if level_scene else None)
    source_edge_count = 0
    transition_png = None
    if original_paint_png is not None:
        transition_png, source_edge_count = source_transition_boundaries(original_paint_png)
        marker=doc["extras"]["original_categorical_terrain_paint"]
        if marker.get("source_boundary_edges") != source_edge_count:
            raise RuntimeError("Original paint boundary count disagrees with source LayerId pixels")
        for target, payload in (
                (layer_id_sidecar_path(destination), original_paint_png),
                (transition_sidecar_path(destination), transition_png)):
            if target.exists() and (not target.is_file() or
                                    target.read_bytes() != payload):
                raise RuntimeError('Original terrain sidecar already exists: ' + str(target))
    if level_scene and combine_meshes:
        raise RuntimeError('Level FBX export must keep source placements separately movable')
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
    # Character material-part naming is not appropriate for original levels.
    # Keep each imported actor/vegetation/terrain tile name in the Blender
    # Outliner; material splitting only adds Blender's normal .001 suffix.
    root_name = (clean_name(source.stem, 'CryEngine Level') if level_scene else
                 name_static_meshes(meshes, source.stem, original_part_labels(source)))
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
    uvless = any(not mesh.data.uv_layers for mesh in meshes)
    validate_blender_meshes(meshes, rigged=any(o.type == 'ARMATURE' for o in visible),
                            embed_textures=embed_textures,
                            allow_uvless=level_scene)
    # Snapshot numeric source data before FBX import destroys Blender objects.
    # Every level export must be reimported, not just static CDF/rig cases.
    audit_meshes = mesh_snapshot(meshes) if level_scene else None
    audit_lights = light_snapshot(source_lights) if level_scene else None
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
        use_tspace=not uvless,
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
                                    require_tangent_space=not uvless,
                                    allow_uvless=level_scene)
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
    audited_geometry_count = 0
    audited_movable = False
    if source_material_ids or native_bounds is not None or level_scene:
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
            audited_geometry_count = verify_mesh_snapshot(
                audit_meshes, mesh_snapshot(roundtrip))
            verify_light_snapshot(audit_lights, light_snapshot(roundtrip_lights))
            audited_movable = verify_independent_movement(
                roundtrip, bpy.context.view_layer)
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
    # Original LayerId pixels are categorical, not reconstructed terrain
    # colors. Blender FBX cannot preserve an unused glTF occlusion texture
    # losslessly; keep the exact source bitmap as a deterministic sidecar.
    if original_paint_png is not None:
        publish_layer_id_sidecar(destination, original_paint_png)
        publish_transition_sidecar(destination, transition_png)
    print(json.dumps({
        "fbx_optimization": optimized,
        "meshes": exported_mesh_count,
        "lights": exported_light_count,
        "roundtrip_verified_meshes": audited_geometry_count,
        "independent_movement_checked": audited_movable,
        "original_paint_boundary_edges": source_edge_count,
        "untextured_geometry_without_uv0": int(uvless),
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
