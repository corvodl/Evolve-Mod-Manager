"""Names and organizes original CryEngine character FBX parts in Blender.

No topology, UV, normal, material or bone data is synthesized here. Mesh
combination is opt-in; incompatible source armatures are never joined.
"""
import json
import re
import struct
from collections import defaultdict
from pathlib import Path

_SOURCE_SLOT = re.compile(r"^CryEngine slot ([0-9]{3}) - ")
_CDF_LABEL = re.compile(
    r"^CryEngine slot [0-9]{3} - CDF part [0-9]{3} (.+?) \| original ID [0-9]{3} \|"
)
_TAG = re.compile(r"^EVOLVE_MID_P[0-9]{3}_S([0-9]{3})__")


def clean_name(value, fallback="Part"):
    if not isinstance(value, str):
        return fallback
    value = re.sub(r"[^A-Za-z0-9_. -]+", "_", value).strip(" ._")
    return value[:56].rstrip(" ._") or fallback


def original_part_labels(source):
    """Return source-authored attachment names by exact material slot.

    glTF material names are written from verified CryEngine CDF geometry part
    spans in static_cdf_material_glb; never infer parts from mesh appearance.
    """
    raw = Path(source).read_bytes()
    if len(raw) < 28 or raw[:4] != b"glTF" or raw[16:20] != b"JSON":
        raise RuntimeError("Hierarchy: invalid source GLB")
    length = struct.unpack_from("<I", raw, 12)[0]
    if length > 24 * 1024 * 1024 or 20 + length > len(raw):
        raise RuntimeError("Hierarchy: oversized source GLB document")
    doc = json.loads(raw[20:20 + length])
    result = {}
    for mat in doc.get("materials", []):
        name = mat.get("name", "")
        slot = _SOURCE_SLOT.match(name)
        if slot:
            part = _CDF_LABEL.match(name)
            result[int(slot.group(1))] = (
                clean_name(part.group(1)) if part else None
            )
    return result


def name_static_meshes(meshes, source_name, slot_labels):
    """Name each separated original material fragment with its CDF part.

    The UV0 preparation step already separates source material IDs, ensuring
    that each fragment's label comes from the exact corresponding source slot.
    """
    seen = defaultdict(int)
    root = clean_name(source_name, "Character")
    for mesh in sorted(meshes, key=lambda obj: obj.name):
        slots = set()
        for poly in mesh.data.polygons:
            mat = mesh.data.materials[poly.material_index]
            match = _TAG.match(mat.name) if mat is not None else None
            if match is not None:
                slots.add(int(match.group(1)))
        if len(slots) > 1:
            raise RuntimeError("Hierarchy: a mesh still spans multiple source materials")
        slot = next(iter(slots), None)
        part = slot_labels.get(slot)
        label = part if part else ("Material_%03d" % slot if slot is not None else "Geometry")
        seen[label] += 1
        suffix = "" if seen[label] == 1 else "_material_%02d" % seen[label]
        mesh.name = clean_name(label + suffix)
        mesh.data.name = mesh.name
    return root


def character_root(name, meshes, armatures=()):
    """Parent all top-level source objects under one exportable Empty.

    Preserve world matrices exactly; skinning meshes stay parented to their
    original armature, while source armatures become children of the root.
    """
    import bpy
    root = bpy.data.objects.new(clean_name(name, "Character"), None)
    root.empty_display_type = 'PLAIN_AXES'
    bpy.context.scene.collection.objects.link(root)
    for obj in list(armatures) + list(meshes):
        if obj.parent is None:
            transform = obj.matrix_world.copy()
            obj.parent = root
            obj.matrix_world = transform
    return root


def combine_source_meshes(meshes, rig_bindings=None, root_name="Character"):
    """Optionally join meshes by *verified identical armature* only.

    Blender remaps material slots and vertex groups by name on join. Validation
    of source MTL IDs, UV loops, normals, texture links and weights is run
    again by callers before publishing the FBX.
    """
    import bpy
    rig_bindings = {} if rig_bindings is None else rig_bindings
    groups = defaultdict(list)
    for obj in meshes:
        groups[rig_bindings.get(obj)].append(obj)
    joined = []
    associations = {}
    for group_index, (rig, group) in enumerate(groups.items()):
        if len(group) == 1:
            active = group[0]
        else:
            bpy.ops.object.select_all(action="DESELECT")
            for obj in group:
                obj.select_set(True)
            active = group[0]
            bpy.context.view_layer.objects.active = active
            bpy.ops.object.join()
        label = clean_name(root_name, "Character") + (
            "_Mesh" if len(groups) == 1 else "_Rig_%03d_Mesh" % group_index
        )
        active.name = label
        active.data.name = label
        joined.append(active)
        if rig is not None:
            associations[active] = rig
    return joined, associations
