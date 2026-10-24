"""Preserve authored CryEngine normals; recover shading only for absent normals.

Source GLBs already carry verified CryEngine vertex normals when available.
Blender's polygons must be smooth to transmit those normals through FBX.
When source normals are unavailable, derive crease edges from authored geometry,
UV corner discontinuities and original material boundaries, without welding,
duplicating vertices or rewriting source UVs/weights/material identities.
"""
import json
import math
import struct
from collections import defaultdict

_CREASE_COS = math.cos(math.radians(60.0))
_UV_EPS = 0.00001


def glb_source_normals(path):
    """Require all material primitives to agree on the source NORMAL stream."""
    with open(path, "rb") as stream:
        header = stream.read(20)
        if len(header) != 20 or header[:4] != b"glTF" or header[16:] != b"JSON":
            raise RuntimeError("Shading source must be a valid GLB")
        length = struct.unpack_from("<I", header, 12)[0]
        if length > 24 * 1024 * 1024:
            raise RuntimeError("Shading source GLB document exceeds the limit")
        doc = json.loads(stream.read(length))
    streams = [
        primitive.get("attributes", {}).get("NORMAL")
        for mesh in doc.get("meshes", [])
        for primitive in mesh.get("primitives", [])
    ]
    if not streams:
        raise RuntimeError("GLB has no source mesh primitives")
    present = [type(accessor) is int and accessor >= 0 for accessor in streams]
    if any(present) != all(present):
        raise RuntimeError("Only some source material fragments have vertex normals")
    return all(present)


def _corner_edges(mesh):
    """Yield original indices and original per-corner UVs for each shared edge."""
    uv = mesh.uv_layers.active or mesh.uv_layers[0]
    links = defaultdict(list)
    for poly in mesh.polygons:
        loops = tuple(poly.loop_indices)
        for at, loop_id in enumerate(loops):
            next_id = loops[(at + 1) % len(loops)]
            v1 = mesh.loops[loop_id].vertex_index
            v2 = mesh.loops[next_id].vertex_index
            if v1 == v2:
                continue
            a = tuple(float(x) for x in uv.data[loop_id].uv)
            b = tuple(float(x) for x in uv.data[next_id].uv)
            key = tuple(sorted((v1, v2)))
            links[key].append((poly, {v1: a, v2: b}))
    return links


def sharp_edge_keys(mesh, crease_cos=_CREASE_COS):
    """Only truly compatible edges smooth; do not join texture seams."""
    if len(mesh.uv_layers) != 1 or not mesh.polygons:
        raise RuntimeError("Shading reconstruction requires one verified UV0 map")
    links = _corner_edges(mesh)
    sharp = set()
    for key, touching in links.items():
        if len(touching) != 2:
            sharp.add(key)  # boundary, nonmanifold or overlapping geometry
            continue
        (left, left_uv), (right, right_uv) = touching
        if left.material_index != right.material_index:
            sharp.add(key)
            continue
        cosine = sum(float(a) * float(b)
                     for a, b in zip(left.normal, right.normal))
        if not math.isfinite(cosine) or cosine < crease_cos:
            sharp.add(key)
            continue
        if any(
            not all(math.isfinite(c) for c in left_uv[vertex] + right_uv[vertex])
            or any(abs(a - b) > _UV_EPS
                   for a, b in zip(left_uv[vertex], right_uv[vertex]))
            for vertex in key
        ):
            sharp.add(key)
    return sharp


def preserve_or_reconstruct_normals(meshes, original_normals):
    """Keep source normals; synthesize UV/crease-safe smooth corners otherwise.

    Blender computes corner normals using smooth polygons and marked sharp
    edges. This preserves the mesh's original vertex IDs and skin assignments.
    The FBX exporter separately computes Mikk-compatible tangent space from
    these loop normals and the original UV0 when use_tspace is enabled.
    """
    if not meshes:
        raise RuntimeError("No Blender meshes to prepare for normal-map shading")
    for obj in meshes:
        mesh = obj.data
        if not mesh.polygons or len(mesh.uv_layers) != 1:
            raise RuntimeError("Source mesh has no original polygons/UV0: " + obj.name)
        vertex_count = len(mesh.vertices)
        face_count = len(mesh.polygons)
        uv_before = [tuple(float(x) for x in corner.uv)
                     for corner in mesh.uv_layers[0].data]
        original_corner_normals = (
            [tuple(float(c) for c in loop.normal) for loop in mesh.loops]
            if original_normals else None
        )
        if original_normals:
            # The glTF importer already provided the source vertex/corner
            # normals. Never average or reorient those vectors.
            for polygon in mesh.polygons:
                polygon.use_smooth = True
        else:
            sharp = sharp_edge_keys(mesh)
            for edge in mesh.edges:
                edge.use_edge_sharp = edge.key in sharp
            for polygon in mesh.polygons:
                polygon.use_smooth = True
        mesh.update()
        if original_corner_normals is not None:
            after = [tuple(float(c) for c in loop.normal) for loop in mesh.loops]
            if len(after) != len(original_corner_normals) or any(
                any(abs(old - new) > 0.001 for old, new in zip(before, current))
                for before, current in zip(original_corner_normals, after)
            ):
                raise RuntimeError(
                    "Blender changed an imported CryEngine corner normal; refusing flat fallback")
        if (len(mesh.vertices) != vertex_count or
                len(mesh.polygons) != face_count or
                uv_before != [tuple(float(x) for x in corner.uv)
                              for corner in mesh.uv_layers[0].data]):
            raise RuntimeError("Normal reconstruction changed original mesh topology or UV0")
        if any(
            not math.isfinite(float(component))
            for loop in mesh.loops
            for component in loop.normal
        ):
            raise RuntimeError("Source/reconstructed loop normal contains nonfinite values")


def normal_samples(meshes):
    """Bounded distribution for verifying FBX round-trip corner normal fidelity."""
    values = [[], [], []]
    for obj in meshes:
        for loop in obj.data.loops:
            for axis in range(3):
                value = float(loop.normal[axis])
                if not math.isfinite(value):
                    raise RuntimeError("Invalid exported corner normal")
                values[axis].append(value)
    return [sorted(axis) for axis in values]


def verify_normal_samples(source, target, tolerance=0.035):
    """FBX can reorder corners but may not flatten or arbitrarily rotate them."""
    for a, b in zip(source, target):
        if not a or len(a) != len(b):
            raise RuntimeError("FBX round-trip lost original/reconstructed corner normals")
        if any(abs(left - right) > tolerance for left, right in zip(a, b)):
            raise RuntimeError("FBX round-trip changed original/reconstructed corner normals")
