"""Blender level FBX round-trip integrity checks.

Compares the imported-source Blender scene with the FBX reimport. A scene can
be visually plausible while having fused, dropped, or displaced objects.
No engine asset bytes are stored here; geometry checks use only live Blender
object coordinates and simple primitive totals.
"""
import math


def _finite_vector(coords):
    values = tuple(float(n) for n in coords)
    if len(values) != 3 or not all(math.isfinite(n) for n in values):
        raise RuntimeError("Level FBX audit: nonfinite object coordinate")
    return values


def mesh_snapshot(meshes):
    """Compact, per-object world geometry record independent of object names.

    Blender can sanitize and suffix object names on import. Source face count
    and world-space bounds make it possible to detect missing or fused meshes
    without relying on those names, material splitting, or object identity.
    """
    records = []
    for obj in meshes:
        if obj.type != "MESH":
            raise RuntimeError("Level FBX audit: expected mesh object")
        polygons = obj.data.polygons
        vertices = obj.data.vertices
        if not polygons or not vertices:
            raise RuntimeError("Level FBX audit: empty scene mesh")
        if any(len(face.vertices) != 3 for face in polygons):
            raise RuntimeError("Level FBX audit: nontriangular scene mesh")
        minimum = [float("inf")] * 3
        maximum = [float("-inf")] * 3
        for vertex in vertices:
            world = _finite_vector(obj.matrix_world @ vertex.co)
            for axis in range(3):
                minimum[axis] = min(minimum[axis], world[axis])
                maximum[axis] = max(maximum[axis], world[axis])
        records.append((len(polygons), tuple(minimum), tuple(maximum),
                        bool(obj.data.uv_layers)))
    return sorted(records, key=lambda row: (row[0], row[1], row[2], row[3]))


def light_snapshot(lights):
    """Record independently transformed local lights, not their display names."""
    records = []
    for obj in lights:
        if obj.type != "LIGHT":
            raise RuntimeError("Level FBX audit: expected Blender light")
        position = _finite_vector(obj.matrix_world.translation)
        records.append(position)
    return sorted(records)


def _close(a, b):
    extent = max(1.0, abs(a), abs(b))
    # FBX float coordinate and Blender axis conversion roundoff.
    return abs(a - b) <= max(0.005, extent * 0.00001)


def verify_mesh_snapshot(source, imported):
    if len(source) != len(imported):
        raise RuntimeError(
            f"Level FBX audit: independent mesh objects changed "
            f"({len(source)} -> {len(imported)})")
    if sum(row[0] for row in source) != sum(row[0] for row in imported):
        raise RuntimeError("Level FBX audit: source triangle count changed")
    for i, (before, after) in enumerate(zip(source, imported)):
        if before[0] != after[0]:
            raise RuntimeError(f"Level FBX audit: object {i} triangle count changed")
        if before[3] and not after[3]:
            raise RuntimeError(f"Level FBX audit: object {i} lost authored UV0")
        for a, b in zip(before[1] + before[2], after[1] + after[2]):
            if not _close(a, b):
                raise RuntimeError(
                    f"Level FBX audit: object {i} moved, rotated, or scaled "
                    f"during FBX round-trip (source={before[1:3]}, "
                    f"imported={after[1:3]})")
    return len(source)


def verify_light_snapshot(source, imported):
    if len(source) != len(imported):
        raise RuntimeError(
            f"Level FBX audit: local light count changed "
            f"({len(source)} -> {len(imported)})")
    for i, (before, after) in enumerate(zip(source, imported)):
        if not all(_close(a, b) for a, b in zip(before, after)):
            raise RuntimeError(
                f"Level FBX audit: local light {i} position changed "
                f"({before} -> {after})")
    return len(source)


def verify_independent_movement(meshes, view_layer):
    """Actively move one imported mesh and ensure a distinct mesh stays put.

    On a real FBX reimport this catches accidentally parenting all placements
    under a single movable mesh. The temporary transform is restored.
    """
    if len(meshes) < 2:
        return False
    first, second = meshes[0], meshes[-1]
    original = first.matrix_world.copy()
    neighbor = _finite_vector(second.matrix_world.translation)
    changed = original.copy()
    changed.translation.x += 1.0
    try:
        first.matrix_world = changed
        view_layer.update()
        new_neighbor = _finite_vector(second.matrix_world.translation)
        if not all(_close(a, b) for a, b in zip(neighbor, new_neighbor)):
            raise RuntimeError(
                "Level FBX audit: moving one mesh changes another placement")
    finally:
        first.matrix_world = original
        view_layer.update()
    return True
