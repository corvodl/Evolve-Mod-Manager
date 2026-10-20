"""Source-faithful UV0 reconstruction ported from private PR #107.
No geometry welding or speculative hole filling; rig/mesh topology remains intact.
Conforms to collaborator Models for painting setup (one UV map, seam-safe repair).
"""
from collections import defaultdict
import math

def reconstruct_uv0(faces, loop_vertices, coordinates, *, vertex_keys=None,
                    epsilon=1e-6, repeat_texture=False):
    """Return (repaired UV corners, diagnostics) without mutating inputs.

    faces: iterable of face loop-index sequences (tris or polygons)
    loop_vertices: vertex id for each loop, or canonical geometric/rig key via
        vertex_keys. Matching identical render-vertex endpoints alone is NOT
        enough to weld a UV seam: BOTH ends of a SHARED edge must match in UV.
    repeat_texture: True only for verified REPEAT texture samplers. Whole
        islands can then move by integer tiles without changing texture sampling.
    """
    coords = tuple(tuple(float(v) for v in uv) for uv in coordinates)
    vertices = tuple(loop_vertices)
    faces = tuple(tuple(face) for face in faces)
    if not 0 < epsilon <= 1e-4:
        raise ValueError('UV tolerance must be between 0 and 1e-4 UV units.')
    if len(vertices) != len(coords):
        raise ValueError('UV loop/vertex stream lengths do not match.')
    if any(len(uv) != 2 or not all(map(math.isfinite, uv)) for uv in coords):
        raise ValueError('Original UV0 includes incomplete/nonfinite coordinates.')
    if any(len(face) < 3 or any(not isinstance(i, int) or i < 0
                               or i >= len(coords) for i in face)
           for face in faces):
        raise ValueError('Invalid UV triangle/face loops.')
    keys = tuple(vertices if vertex_keys is None else vertex_keys)
    if len(keys) != len(vertices):
        raise ValueError('Canonical render-vertex keys do not cover all loops.')
    size = len(coords)
    parent = list(range(size))
    def find(i):
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i
    def union(a, b):
        pa, pb = find(a), find(b)
        if pa != pb:
            parent[pb] = pa
    # An edge may join two triangles even when CryEngine emitted duplicate
    # render vertices at normals/UV seams. Require exactly two neighboring
    # faces and compatible endpoint UVs to avoid joining mirror borders.
    edges = defaultdict(list)
    for face in faces:
        for j, a in enumerate(face):
            b = face[(j+1) % len(face)]
            va, vb = keys[a], keys[b]
            if va == vb:
                continue
            order = (va, vb) if repr(va) < repr(vb) else (vb, va)
            edges[order].append((va, a, vb, b))
    matched_edges = 0
    continuous_pairs = []
    for neighbors in edges.values():
        if len(neighbors) != 2:
            continue  # boundary, overlapping duplicate faces, nonmanifold
        first, second = neighbors
        first_uv = {first[0]: first[1], first[2]: first[3]}
        second_uv = {second[0]: second[1], second[2]: second[3]}
        if (set(first_uv) != set(second_uv)
                or any(math.dist(coords[first_uv[k]], coords[second_uv[k]])
                       > epsilon for k in first_uv)):
            continue
        matched_edges += 1
        for k in first_uv:
            union(first_uv[k], second_uv[k])
            continuous_pairs.append((first_uv[k], second_uv[k]))
    # Small numeric cracks on CONNECTED corners may be snapped. Do not
    # accumulate transitive approximation drift across a large island.
    groups = defaultdict(list)
    for i in range(size):
        groups[find(i)].append(i)
    result = list(coords)
    snaps = 0
    for ids in groups.values():
        if len(ids) < 2:
            continue
        us = [coords[i][0] for i in ids]
        vs = [coords[i][1] for i in ids]
        if max(us)-min(us) > epsilon or max(vs)-min(vs) > epsilon:
            continue
        # Do not average already matching floating-point UVs. Repeated
        # coordinates from actual Lennox/Sunny/Parasite Host source meshes
        # accumulated rounding drift when averaged, even when no repair
        # was needed. Reuse the most common ORIGINAL corner exactly and
        # move only minority near-equal corners to that coordinate.
        values = defaultdict(int)
        ordered = []
        for i in ids:
            uv = coords[i]
            if uv not in values:
                ordered.append(uv)
            values[uv] += 1
        if len(values) == 1:
            continue
        canonical = max(ordered, key=lambda uv: values[uv])
        for i in ids:
            if coords[i] != canonical:
                result[i] = canonical
                snaps += 1
    # Whole polygons belong to one UV island. A separate connectivity tree
    # prevents accidentally moving each vertex of an isolated triangle by
    # a different tile shift (which would scramble its texture mapping).
    island_parent = list(range(size))
    def island_find(i):
        while island_parent[i] != i:
            island_parent[i] = island_parent[island_parent[i]]
            i = island_parent[i]
        return i
    def island_union(a, b):
        a, b = island_find(a), island_find(b)
        if a != b:
            island_parent[b] = a
    for face in faces:
        for corner in face[1:]:
            island_union(face[0], corner)
    for a, b in continuous_pairs:
        island_union(a, b)
    islands = defaultdict(list)
    for i in range(size):
        islands[island_find(i)].append(i)
    moved, outside = 0, 0
    if repeat_texture:
        for ids in islands.values():
            shifts = []
            for axis in (0, 1):
                vals = [result[i][axis] for i in ids]
                lo, hi = min(vals), max(vals)
                if lo >= -epsilon and hi <= 1+epsilon:
                    shifts.append(0)
                    continue
                candidate = math.floor(lo)
                shift = -candidate
                if (lo+shift >= -epsilon and hi+shift <= 1+epsilon
                        and abs(shift) <= 100):
                    shifts.append(shift)
                else:
                    shifts.append(0)
                    outside += 1
            if any(shifts):
                moved += 1
                for i in ids:
                    result[i] = (result[i][0]+shifts[0],
                                 result[i][1]+shifts[1])
    return tuple(result), {
        'uv_sets': 1, 'islands': len(islands),
        'shared_edges_verified': matched_edges,
        'snapped_corners': snaps, 'shifted_repeat_islands': moved,
        'unshiftable_axes': outside, 'repeat_safe': bool(repeat_texture),
        'tolerance_uv': epsilon, 'changed': tuple(result) != coords,
    }



def _repair_original_uv(obj):
    mesh = obj.data
    # Blender's UV layers collection has no active_render attribute; its
    # active layer is the reliable default unless a material names another.
    active = mesh.uv_layers.active
    tex_nodes = []
    referenced = set()
    for material in mesh.materials:
        if not material or not material.use_nodes:
            continue
        for node in material.node_tree.nodes:
            if node.type != 'TEX_IMAGE':
                continue
            tex_nodes.append(node)
            vector = node.inputs.get('Vector')
            if vector and vector.is_linked:
                upstream = vector.links[0].from_node
                if upstream.type == 'UVMAP':
                    referenced.add(upstream.uv_map)
                elif upstream.type == 'MAPPING':
                    # Mapping modifiers can invalidate integer UV tile shifts;
                    # still allow the original unshifted atlas.
                    for vector_input in upstream.inputs:
                        if vector_input.name == 'Vector' and vector_input.is_linked:
                            source = vector_input.links[0].from_node
                            if source.type == 'UVMAP':
                                referenced.add(source.uv_map)
    if len(referenced) > 1:
        raise RuntimeError('Material uses multiple source UV channels. '
                           'One-channel painting export would change texturing: ' + obj.name)
    if referenced:
        selected = mesh.uv_layers.get(next(iter(referenced)))
        if selected is None:
            raise RuntimeError('Source texture references missing UV map: ' + obj.name)
    else:
        selected = active or (mesh.uv_layers[0] if mesh.uv_layers else None)
    if selected is None:
        if tex_nodes:
            raise RuntimeError('Textured mesh has no source UV coordinates: ' + obj.name)
        return {'object': obj.name, 'uv_sets': 0, 'note':
                'No source UV map; no synthetic UVs created.',
                'topology_preserved': True, 'weights_preserved': True}
    if len(selected.data) != len(mesh.loops):
        raise RuntimeError('Incomplete original UV stream: ' + obj.name)
    original = tuple(tuple(float(coord) for coord in loop.uv)
                     for loop in selected.data)
    if any(not math.isfinite(value) for uv in original for value in uv):
        raise RuntimeError('Invalid source UV coordinates: ' + obj.name)
    vertices_before = [(tuple(v.co), tuple(sorted(
        (g.group, round(g.weight, 9)) for g in v.groups)))
        for v in mesh.vertices]
    faces_before = tuple(tuple(face.vertices) for face in mesh.polygons)
    # Material/rig-bound vertex fingerprint: recognize exact duplicate cooked
    # render vertices without welding geometry or different bone assignments.
    keys = [(
        tuple(round(float(k), 8) for k in v.co),
        tuple(sorted((g.group, round(g.weight, 8)) for g in v.groups)))
        for v in mesh.vertices]
    loop_keys = [keys[loop.vertex_index] for loop in mesh.loops]
    faces = [tuple(face.loop_indices) for face in mesh.polygons]
    # Whole-island integer shifts are texture-equivalent only for REPEAT, and
    # are disabled for UDIM, transformed, clamped or unmapped samplers.
    is_repeat = bool(tex_nodes) and all(
        getattr(node, 'extension', '') == 'REPEAT'
        and not (node.image and node.image.source == 'TILED')
        and (not node.inputs.get('Vector') or
             not node.inputs['Vector'].is_linked or
             node.inputs['Vector'].links[0].from_node.type == 'UVMAP')
        for node in tex_nodes)
    corrected, stats = reconstruct_uv0(
        faces, [loop.vertex_index for loop in mesh.loops], original,
        vertex_keys=loop_keys, epsilon=1e-6, repeat_texture=is_repeat)
    for uv_loop, value in zip(selected.data, corrected):
        uv_loop.uv = value
    # A mesh is split by material before this step. Remove additional maps
    # only when all texture nodes resolve to the surviving source map.
    source_names = {selected.name}
    for node in tex_nodes:
        vector = node.inputs.get('Vector')
        if vector and vector.is_linked:
            upstream = vector.links[0].from_node
            if upstream.type == 'UVMAP' and upstream.uv_map not in source_names:
                raise RuntimeError('Texture still depends on another UV set: ' + obj.name)
    for layer in tuple(mesh.uv_layers):
        if layer != selected:
            mesh.uv_layers.remove(layer)
    old_name = selected.name
    selected.name = 'UV0_Original'
    mesh.uv_layers.active = selected
    selected.active_render = True
    for material in mesh.materials:
        if material and material.use_nodes:
            for node in material.node_tree.nodes:
                if node.type == 'UVMAP' and node.uv_map == old_name:
                    node.uv_map = 'UV0_Original'
    after_vertices = [(tuple(v.co), tuple(sorted(
        (g.group, round(g.weight, 9)) for g in v.groups)))
        for v in mesh.vertices]
    if (after_vertices != vertices_before or
            tuple(tuple(face.vertices) for face in mesh.polygons) != faces_before):
        raise RuntimeError('UV repair modified rigged render topology: ' + obj.name)
    if len(mesh.uv_layers) != 1 or len(selected.data) != len(mesh.loops):
        raise RuntimeError('Painting model did not retain exactly one UV set.')
    return dict(stats, object=obj.name, original='UV0_Original',
                topology_preserved=True, weights_preserved=True,
                uv_loops=len(corrected), uv_sets=1)


def prepare_original_uv(meshes):
    """Split original material groups, then repair source UV corners per mesh."""
    import bpy
    original = set(meshes)
    existing = set(bpy.data.objects)
    for obj in tuple(meshes):
        if not obj.data.polygons:
            raise RuntimeError("Painting export cannot include empty mesh: " + obj.name)
        material_ids = {poly.material_index for poly in obj.data.polygons}
        if len(material_ids) <= 1:
            continue
        bpy.ops.object.select_all(action="DESELECT")
        obj.select_set(True)
        bpy.context.view_layer.objects.active = obj
        bpy.ops.object.mode_set(mode="EDIT")
        bpy.ops.mesh.select_all(action="SELECT")
        bpy.ops.mesh.separate(type="MATERIAL")
        bpy.ops.object.mode_set(mode="OBJECT")
    result = [obj for obj in bpy.data.objects
              if obj.type == "MESH" and (obj in original or obj not in existing)]
    if not result:
        raise RuntimeError("No drawable material-specific meshes remain")
    for obj in result:
        used = {poly.material_index for poly in obj.data.polygons}
        if len(used) != 1:
            raise RuntimeError("Mixed source material/UV sets after separation: " + obj.name)
        # Shared source material nodes must not be renamed on unrelated meshes.
        for index, material in enumerate(tuple(obj.data.materials)):
            if material and material.use_nodes:
                obj.data.materials[index] = material.copy()
        _repair_original_uv(obj)
    return result
