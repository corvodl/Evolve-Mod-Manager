"""Preserve verified CryEngine subset IDs across Blender GLB->FBX conversions.

Original source face counts are carried in the Rust CDF manifest. Naming a
material by its authored MTL leaf index (not its position in Blender's compact
material list) means empty and NoDraw leaves cannot shift later assignments.
Every original material-to-face assignment is audited after FBX round-trip.
"""
from collections import Counter
import re

_SOURCE = re.compile(r"^CryEngine slot ([0-9]{3}) - ")
_TAG = re.compile(r"^EVOLVE_MID_P([0-9]{3})_S([0-9]{3})__")


def _check(condition, message):
    if not condition:
        raise RuntimeError("CryEngine material IDs: " + message)


def parse_material_records(records):
    _check(isinstance(records, list) and 1 <= len(records) <= 128,
           "missing or excessive source material IDs")
    result = {}
    for entry in records:
        _check(isinstance(entry, dict), "invalid original material entry")
        identity, count, name = (entry.get("id"),
                                 entry.get("triangles"), entry.get("name"))
        _check(type(identity) is int and 0 <= identity < 128,
               "invalid original material ID")
        _check(type(count) is int and 0 < count <= 5_000_000,
               "invalid original material triangle count")
        _check(isinstance(name, str) and len(name) <= 256
               and not any(ord(ch) < 32 for ch in name),
               "invalid original material name")
        _check(identity not in result, "duplicate original material ID")
        result[identity] = count
    return result


def _assigned_material(mesh, polygon):
    index = polygon.material_index
    _check(0 <= index < len(mesh.data.materials),
           "missing material index on " + mesh.name)
    material = mesh.data.materials[index]
    _check(material is not None,
           "unassigned source material on " + mesh.name)
    return material


def _triangles(mesh, polygon):
    count = len(polygon.vertices) - 2
    _check(count == 1, "nontriangular source/FBX mesh face on " + mesh.name)
    return count


def _material_id(name):
    match = _SOURCE.match(name)
    return int(match.group(1)) if match else None


def _tagged_id(name):
    match = _TAG.match(name)
    _check(match is not None, "material identity was lost: " + name)
    return int(match.group(1)), int(match.group(2))


def source_material_histogram(meshes):
    """Count triangles by (part index, original MTL leaf ID)."""
    histogram = Counter()
    for mesh in meshes:
        _check(bool(mesh.data.polygons),
               "empty exported mesh " + mesh.name)
        for polygon in mesh.data.polygons:
            material = _assigned_material(mesh, polygon)
            histogram[_tagged_id(material.name)] += _triangles(mesh, polygon)
    return dict(histogram)


def verify_original_material_ids(meshes, expected_parts):
    _check(isinstance(expected_parts, list) and bool(expected_parts),
           "missing source part manifest")
    expected = {}
    for part_index, part in enumerate(expected_parts):
        _check(part_index < 128, "too many source parts")
        for identity, triangles in parse_material_records(part).items():
            expected[(part_index, identity)] = triangles
    actual = source_material_histogram(meshes)
    _check(actual == expected,
           "FBX reassigned a source material ID or changed its face count: "
           + "expected " + str(sorted(expected.items()))
           + ", got " + str(sorted(actual.items())))
    return actual


def bind_original_material_ids(meshes, part_index, part_name, records):
    """Assign stable per-part ID tags before UV/material separation.

    An original multi-slot GLB must have named 'CryEngine slot NNN' leaves.
    Only a genuine one-slot mesh may infer the sole material identity.
    Blender material positions are never treated as original material IDs.
    """
    _check(type(part_index) is int and 0 <= part_index < 128,
           "invalid source part index")
    _check(isinstance(part_name, str) and part_name,
           "invalid source part name")
    expected = parse_material_records(records)
    _check(bool(meshes), "source has no meshes")
    for mesh in meshes:
        _check(bool(mesh.data.polygons),
               "source mesh has no polygons: " + mesh.name)
        used = {polygon.material_index for polygon in mesh.data.polygons}
        _check(all(isinstance(i, int) and i >= 0 for i in used),
               "invalid source material index")
        if not mesh.data.materials and len(expected) == 1:
            # A source with one implicit ID can lack authored material maps.
            # This does not invent a new ID; Rust already proved the one ID.
            import bpy
            mat = bpy.data.materials.new(name="Source material without MTL")
            mat.use_nodes = True
            mesh.data.materials.append(mat)
        for local_index in used:
            _check(local_index < len(mesh.data.materials),
                   "unmapped material on " + mesh.name)
            source = mesh.data.materials[local_index]
            _check(source is not None,
                   "empty material slot on " + mesh.name)
            source_id = _material_id(source.name)
            if source_id is None:
                _check(len(expected) == 1,
                       "original multi-material leaf ID missing: " + source.name)
                source_id = next(iter(expected))
            _check(source_id in expected,
                   "material ID " + str(source_id) + " not in original subsets")
            # Make a distinct material per original part so Blender's global
            # deduplication or generated .001 suffixes cannot conflate leaves.
            unique = source.copy()
            label = re.sub(r"[^a-zA-Z0-9_.-]+", "_", source.name).strip("_")
            prefix = f"EVOLVE_MID_P{part_index:03}_S{source_id:03}__"
            unique.name = prefix + (label[:24] or "Material")
            mesh.data.materials[local_index] = unique
    subset = source_material_histogram(meshes)
    actual = {identity: triangles for (part, identity), triangles
              in subset.items() if part == part_index}
    _check(actual == expected,
           "original source subset face counts do not match imported GLB "
           + str(part_name) + ": " + str(actual) + " != " + str(expected))
    return subset
