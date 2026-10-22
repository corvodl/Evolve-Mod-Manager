"""Verify static CryEngine Z-up positions survive glTF and FBX transforms.

GLB geometry attributes are in native CryEngine world coordinates. The static
FBX bridge inserts one -90° X rotation to compensate Blender's Y-up glTF
conversion; world-coordinate bounds must match the untouched POSITION accessor.
This verifier deliberately does not use height/width heuristics: weapons and
prone animations can legitimately be longer on either horizontal axis.
"""
import json
import math
import struct
from pathlib import Path

MARKER = "cryengine_z_up_to_gltf_y_up"


def original_static_bounds(glb_file):
    with Path(glb_file).open("rb") as stream:
        header = stream.read(20)
        if len(header) < 20 or header[:4] != b"glTF":
            raise RuntimeError("Invalid intermediate GLB header")
        version, total, size = struct.unpack_from("<III", header, 4)
        if version != 2 or size < 4 or size > 64 * 1024 * 1024 or header[16:20] != b"JSON":
            raise RuntimeError("Invalid intermediate GLB JSON chunk")
        document = json.loads(stream.read(size))
    marker = document.get("extras", {}).get("evolve_static_fbx_basis")
    if marker is None:
        return None  # Other verified GLB routes retain their own transforms.
    if marker != MARKER:
        raise RuntimeError("Unrecognized CryEngine static FBX axis convention")
    # A source-verified static FBX has one native-position source mesh and
    # one explicit basis parent. Never infer an angle from bounding dimensions.
    scene_index = document.get("scene")
    scenes, nodes = document.get("scenes", []), document.get("nodes", [])
    if type(scene_index) is not int or not (0 <= scene_index < len(scenes)):
        raise RuntimeError("Missing source scene")
    roots = scenes[scene_index].get("nodes")
    if not isinstance(roots, list) or len(roots) != 1:
        raise RuntimeError("Static source must have one basis root")
    basis_id = roots[0]
    if type(basis_id) is not int or not (0 <= basis_id < len(nodes)):
        raise RuntimeError("Invalid static basis root")
    basis = nodes[basis_id]
    quat = basis.get("rotation", [])
    if (not isinstance(quat, list) or len(quat) != 4 or
            any(type(q) not in (int, float) or not math.isfinite(q) for q in quat)):
        raise RuntimeError("Missing static source basis quaternion")
    s = math.sqrt(0.5)
    if any(abs(a - b) > 1e-5 for a, b in zip(quat, (-s, 0, 0, s))):
        raise RuntimeError("Static source has wrong CryEngine Z-up basis")
    children = basis.get("children", [])
    if len(children) != 1 or nodes[children[0]].get("mesh") != 0:
        raise RuntimeError("Static source basis must parent the actual geometry")
    positions = []
    for primitive in document["meshes"][0]["primitives"]:
        accessor_id = primitive["attributes"]["POSITION"]
        accessor = document["accessors"][accessor_id]
        if not all(k in accessor for k in ("min", "max")):
            raise RuntimeError("Source POSITION accessor lacks authored bounds")
        minimum, maximum = accessor["min"], accessor["max"]
        if (len(minimum) != 3 or len(maximum) != 3 or
                any(not math.isfinite(float(v)) for v in minimum + maximum)):
            raise RuntimeError("Nonfinite native CryEngine bounds")
        positions.append((minimum, maximum))
    if not positions:
        raise RuntimeError("Static GLB lacks original geometry")
    return ([min(box[0][axis] for box in positions) for axis in range(3)],
            [max(box[1][axis] for box in positions) for axis in range(3)])


def world_mesh_bounds(meshes):
    lower = [float("inf")] * 3
    upper = [float("-inf")] * 3
    count = 0
    for obj in meshes:
        for vertex in obj.data.vertices:
            coord = obj.matrix_world @ vertex.co
            for axis in range(3):
                value = float(coord[axis])
                if not math.isfinite(value):
                    raise RuntimeError("Nonfinite FBX world-space vertex")
                lower[axis] = min(lower[axis], value)
                upper[axis] = max(upper[axis], value)
            count += 1
    if count < 3:
        raise RuntimeError("Static FBX import has no geometry")
    return lower, upper


def verify_static_world_bounds(meshes, original, stage):
    actual = world_mesh_bounds(meshes)
    extent = max(1., *(original[1][axis] - original[0][axis]
                       for axis in range(3)))
    tolerance = max(0.001, extent * 0.0005)
    for axis in range(3):
        if (abs(actual[0][axis] - original[0][axis]) > tolerance or
                abs(actual[1][axis] - original[1][axis]) > tolerance):
            raise RuntimeError(
                "Static CryEngine geometry was rotated, scaled, or displaced "
                "during " + stage + ": native bounds " + repr(original) +
                ", Blender bounds " + repr(actual))
    return True
