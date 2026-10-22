"""Post-export FBX safety gate. Standard library only: Blender is used solely by scene helpers.

Never modify geometry, UV arrays, bone models, or weighted cluster transforms.
FBX 7.4 (32-bit offsets) and 7.5+ (64-bit offsets) are supported.
"""
import math
import os
import struct
import zlib
from pathlib import Path

MAGIC = b"Kaydara FBX Binary  \x00\x1a\x00"
MAX_BINARY = 1024 * 1024 * 1024


class FBXValidationError(RuntimeError):
    pass


class Node:
    def __init__(self, name, props=b"", count=0, children=(), terminator=False):
        self.name, self.props, self.count = name, props, count
        self.children = list(children)
        self.terminator = terminator

    def get(self, name):
        return next((child for child in self.children if child.name == name), None)

    def children_named(self, name):
        return [child for child in self.children if child.name == name]


def _properties(node):
    result, pos, raw = [], 0, node.props
    for _ in range(node.count):
        if pos >= len(raw):
            raise FBXValidationError("Truncated FBX property list")
        typ = raw[pos:pos+1]
        pos += 1
        size = {b"Y": 2, b"C": 1, b"I": 4, b"F": 4,
                b"D": 8, b"L": 8}.get(typ)
        if size is not None:
            if pos + size > len(raw):
                raise FBXValidationError("Truncated FBX scalar")
            value = raw[pos:pos+size]
            if typ == b"L":
                result.append(struct.unpack("<q", value)[0])
            elif typ == b"I":
                result.append(struct.unpack("<i", value)[0])
            else:
                result.append(value)
            pos += size
        elif typ in (b"S", b"R"):
            if pos + 4 > len(raw):
                raise FBXValidationError("Truncated FBX string")
            length = struct.unpack_from("<I", raw, pos)[0]
            pos += 4
            if pos + length > len(raw):
                raise FBXValidationError("Invalid FBX string length")
            result.append(raw[pos:pos+length])
            pos += length
        elif typ in (b"b", b"c", b"i", b"l", b"f", b"d"):
            if pos + 12 > len(raw):
                raise FBXValidationError("Truncated FBX array")
            count, encoding, length = struct.unpack_from("<III", raw, pos)
            pos += 12
            if pos + length > len(raw) or count > 100_000_000:
                raise FBXValidationError("Invalid FBX array length")
            result.append((typ, count, encoding, raw[pos:pos+length]))
            pos += length
        else:
            raise FBXValidationError("Unknown FBX property type: " + repr(typ))
    if pos != len(raw):
        raise FBXValidationError("Trailing bytes in FBX property list")
    return result


def _numeric_array(node, type_codes, max_values=20_000_000):
    props = _properties(node)
    if len(props) != 1 or not isinstance(props[0], tuple):
        raise FBXValidationError("Missing FBX numeric array: " + str(node.name))
    typ, count, encoding, payload = props[0]
    sizes = {b"i": ("i", 4), b"l": ("q", 8),
             b"d": ("d", 8), b"f": ("f", 4)}
    if typ not in type_codes or typ not in sizes or count > max_values:
        raise FBXValidationError("Unsupported FBX numeric array")
    code, width = sizes[typ]
    if encoding == 1:
        payload = zlib.decompress(payload)
    elif encoding != 0:
        raise FBXValidationError("Unsupported FBX array compression")
    if len(payload) != count * width:
        raise FBXValidationError("Invalid FBX array payload length")
    return struct.unpack("<" + str(count) + code, payload) if count else ()


def _read_nodes(data):
    if len(data) < 64 or not data.startswith(MAGIC):
        raise FBXValidationError("Not a binary FBX")
    version = struct.unpack_from("<I", data, len(MAGIC))[0]
    if not 7400 <= version < 8000:
        raise FBXValidationError("Unsupported binary FBX version")
    large = version >= 7500
    fmt = "<QQQB" if large else "<IIIB"
    size = struct.calcsize(fmt)
    empty = size
    end_data = len(data)

    def read(pos, depth=0):
        if depth > 80 or pos + size > end_data:
            raise FBXValidationError("Malformed FBX node nesting")
        end, count, length, nlen = struct.unpack_from(fmt, data, pos)
        start = pos + size
        props_start = start + nlen
        props_end = props_start + length
        if (end <= pos or end > end_data or props_end > end or
                nlen > 255 or count > 100000):
            raise FBXValidationError("Invalid FBX node offsets")
        node = Node(data[start:props_start], data[props_start:props_end], count)
        pos = props_end
        while pos < end:
            if pos + empty <= end and data[pos:pos+empty] == bytes(empty):
                pos += empty
                node.terminator = True
                break
            child, pos = read(pos, depth+1)
            node.children.append(child)
        if pos != end:
            raise FBXValidationError("Invalid FBX nested node boundary")
        return node, end

    pos = len(MAGIC) + 4
    roots = []
    while pos + empty <= end_data and data[pos:pos+empty] != bytes(empty):
        node, pos = read(pos)
        roots.append(node)
    if pos + empty > end_data:
        raise FBXValidationError("Missing FBX footer separator")
    return version, roots, data[pos:]


def _write_nodes(version, roots, footer):
    fmt = "<QQQB" if version >= 7500 else "<IIIB"
    empty = bytes(struct.calcsize(fmt))
    size = len(empty)

    def write(node, pos):
        if len(node.name) > 255:
            raise FBXValidationError("FBX node name exceeds limit")
        children = []
        next_pos = pos + size + len(node.name) + len(node.props)
        for child in node.children:
            encoded = write(child, next_pos)
            children.append(encoded)
            next_pos += len(encoded)
        if node.terminator or node.children:
            children.append(empty)
            next_pos += size
        try:
            header = struct.pack(fmt, next_pos, node.count,
                                 len(node.props), len(node.name))
        except struct.error as exc:
            raise FBXValidationError("FBX node offset overflow") from exc
        return header + node.name + node.props + b"".join(children)

    pieces, pos = [MAGIC, struct.pack("<I", version)], len(MAGIC)+4
    for root in roots:
        encoded = write(root, pos)
        pieces.append(encoded)
        pos += len(encoded)
    return b"".join(pieces) + footer


def _object_id(node):
    props = _properties(node)
    if not props or not isinstance(props[0], int):
        raise FBXValidationError("FBX object has no ID")
    return props[0]


def _text(node):
    props = _properties(node)
    return props[0] if len(props) == 1 and isinstance(props[0], bytes) else b""


def _pose_nodes(pose):
    if pose.get(b"Type") is None or _text(pose.get(b"Type")) != b"BindPose":
        return None
    entries = {}
    for item in pose.children_named(b"PoseNode"):
        ref, matrix = item.get(b"Node"), item.get(b"Matrix")
        if ref is None or matrix is None:
            return None
        ref_id = _properties(ref)[0]
        if not isinstance(ref_id, int) or ref_id in entries:
            return None
        entries[ref_id] = (item, _properties(matrix))
    return entries


def _set_count(node, value):
    node.props = b"I" + struct.pack("<i", value)
    node.count = 1


def _validate_geometry(objects):
    mesh_count = 0
    for mesh in objects.children_named(b"Geometry"):
        props = _properties(mesh)
        if len(props) < 3 or props[2] != b"Mesh":
            continue
        mesh_count += 1
        vertex_node = mesh.get(b"Vertices")
        if vertex_node is None:
            raise FBXValidationError("FBX mesh missing vertex coordinates")
        vertices = _numeric_array(vertex_node, {b"d", b"f"})
        if not vertices or len(vertices) % 3 or not all(map(math.isfinite, vertices)):
            raise FBXValidationError("Malformed FBX mesh vertices")
        layers = mesh.children_named(b"LayerElementUV")
        if not layers:
            raise FBXValidationError("FBX mesh missing UV layers")
        for layer in layers:
            uv_node = layer.get(b"UV")
            if uv_node is None:
                raise FBXValidationError("FBX missing UV coordinate array")
            uv = _numeric_array(uv_node, {b"d", b"f"})
            if not uv or len(uv) % 2 or not all(map(math.isfinite, uv)):
                raise FBXValidationError("Nonfinite/incomplete FBX UV coordinates")
            uv_index = layer.get(b"UVIndex")
            if uv_index is not None:
                indices = _numeric_array(uv_index, {b"i", b"l"})
                if not indices or any(idx < 0 or idx >= len(uv)//2 for idx in indices):
                    raise FBXValidationError("Out-of-range FBX UV indices")
    if not mesh_count:
        raise FBXValidationError("FBX contains no meshes")
    return mesh_count


def _cluster_weights(cluster):
    indices_node, weights_node = cluster.get(b"Indexes"), cluster.get(b"Weights")
    if indices_node is None or weights_node is None:
        raise FBXValidationError("FBX cluster missing indices/weights")
    indices = _numeric_array(indices_node, {b"i", b"l"})
    weights = _numeric_array(weights_node, {b"d", b"f"})
    if len(indices) != len(weights):
        raise FBXValidationError("FBX cluster has mismatched weight indices")
    if any(i < 0 for i in indices) or any(not math.isfinite(w) or w < 0 for w in weights):
        raise FBXValidationError("FBX cluster contains invalid weights")
    return len(indices), any(w > 0 for w in weights)


def optimize_binary_fbx(path, embed_textures=True):
    """Prune zero-weight clusters and merge only compatible bind poses.

    Failure leaves the original staged FBX untouched; caller must publish only
    after the optimized file has passed the FBX and Blender round-trip checks.
    """
    path = Path(path)
    if not path.is_file() or path.stat().st_size > MAX_BINARY:
        raise FBXValidationError("FBX input missing or larger than 1 GiB")
    version, roots, footer = _read_nodes(path.read_bytes())
    objects = next((node for node in roots if node.name == b"Objects"), None)
    connections = next((node for node in roots if node.name == b"Connections"), None)
    if objects is None or connections is None:
        raise FBXValidationError("Missing FBX object/connection tables")
    meshes = _validate_geometry(objects)
    if not embed_textures and any(node.name in (b"Texture", b"Video")
                                  for node in objects.children):
        raise FBXValidationError("Untextured FBX still references images")

    clusters = [obj for obj in objects.children_named(b"Deformer")
                if len(_properties(obj)) >= 3 and _properties(obj)[2] == b"Cluster"]
    removed = set()
    active_clusters = 0
    for cluster in clusters:
        _, weighted = _cluster_weights(cluster)
        if weighted:
            active_clusters += 1
        else:
            removed.add(_object_id(cluster))
    if len(clusters) and not active_clusters:
        raise FBXValidationError("Rig contains no weighted clusters")

    # FBX bind poses have no connections. Merge only poses sharing unchanged
    # node IDs and identical bind transforms. Require identical node sets:\n    # partial overlaps can be legitimate distinct historical bind poses.
    # separate. This collects one pose per armature instead of one per mesh.
    poses = objects.children_named(b"Pose")
    pose_groups = []
    for pose in poses:
        entries = _pose_nodes(pose)
        if entries is None:
            continue
        combined = False
        for master, known in pose_groups:
            overlap = set(known) & set(entries)
            if set(known) != set(entries) or any(known[i][1] != entries[i][1] for i in overlap):
                continue
            # Existing Pose ID must be unused in connections before deleting.
            if _object_id(pose) in _connected_ids(connections):
                continue
            for key, value in entries.items():
                if key not in known:
                    known[key] = value
                    master.children.append(value[0])
            nb = master.get(b"NbPoseNodes")
            if nb is None:
                raise FBXValidationError("Bind pose missing node count")
            _set_count(nb, len(known))
            removed.add(_object_id(pose))
            combined = True
            break
        if not combined:
            pose_groups.append((pose, dict(entries)))

    # Object IDs outside Deformer/Pose and their connections are preserved.
    objects.children = [node for node in objects.children
                        if node.name not in (b"Deformer", b"Pose")
                        or _object_id(node) not in removed]
    connections.children = [
        link for link in connections.children
        if not (link.name == b"C" and len(_properties(link)) >= 3
                and (_properties(link)[1] in removed or _properties(link)[2] in removed))
    ]

    definitions = next((n for n in roots if n.name == b"Definitions"), None)
    if definitions:
        for obj_type in definitions.children_named(b"ObjectType"):
            type_name = _properties(obj_type)
            if type_name and type_name[0] in (b"Deformer", b"Pose"):
                count_node = obj_type.get(b"Count")
                if count_node:
                    _set_count(count_node, sum(obj.name == type_name[0]
                                               for obj in objects.children))

    encoded = _write_nodes(version, roots, footer)
    check_version, check_roots, _ = _read_nodes(encoded)
    if check_version != version:
        raise FBXValidationError("FBX version changed during optimization")
    check_objects = next(n for n in check_roots if n.name == b"Objects")
    _validate_geometry(check_objects)
    surviving = [c for c in check_objects.children_named(b"Deformer")
                 if len(_properties(c)) >= 3 and _properties(c)[2] == b"Cluster"]
    if len(surviving) != active_clusters or any(not _cluster_weights(c)[1] for c in surviving):
        raise FBXValidationError("Weighted FBX cluster loss")
    if _connected_ids(next(n for n in check_roots if n.name == b"Connections")) & removed:
        raise FBXValidationError("Dangling FBX object connection")
    # Atomic replacement of the staged file (never of a user destination).
    target = path.with_name(path.name + ".optimized")
    try:
        with target.open("xb") as fp:
            fp.write(encoded)
            fp.flush()
            os.fsync(fp.fileno())
        os.replace(target, path)
    finally:
        if target.exists():
            target.unlink()
    return {"meshes": meshes, "weighted_clusters": active_clusters,
            "removed_empty_clusters": len(clusters) - active_clusters,
            "merged_bind_poses": len(poses) - len(pose_groups)}


def _connected_ids(connections):
    result = set()
    for link in connections.children_named(b"C"):
        values = _properties(link)
        if len(values) >= 3:
            if isinstance(values[1], int):
                result.add(values[1])
            if isinstance(values[2], int):
                result.add(values[2])
    return result


def prepare_export_materials(meshes, embed_textures):
    """No embedded textures means *no* stale temporary file references."""
    for mesh in meshes:
        for material in mesh.data.materials:
            if material is None:
                continue
            if not embed_textures:
                material.diffuse_color = (1.0, 1.0, 1.0, 1.0)
                material.use_nodes = True
                tree = material.node_tree
                tree.nodes.clear()
                output = tree.nodes.new("ShaderNodeOutputMaterial")
                surface = tree.nodes.new("ShaderNodeBsdfPrincipled")
                surface.inputs["Base Color"].default_value = (1., 1., 1., 1.)
                surface.inputs["Metallic"].default_value = 0.
                surface.inputs["Roughness"].default_value = .5
                tree.links.new(surface.outputs["BSDF"], output.inputs["Surface"])
            elif material.use_nodes:
                for node in material.node_tree.nodes:
                    if node.type != "TEX_IMAGE":
                        continue
                    image = node.image
                    if image is None or not (image.packed_file or
                            (image.filepath and Path(__import__("bpy").path.abspath(image.filepath)).is_file())):
                        raise FBXValidationError(
                            "Embedded export has unresolved image: " + material.name)


def validate_blender_meshes(meshes, *, rigged=False, embed_textures=True):
    """Pre-export UV, material and skin-weight checks after UV0 repair."""
    if not meshes:
        raise FBXValidationError("No exported meshes")
    for mesh in meshes:
        data = mesh.data
        if not data.polygons or not data.uv_layers:
            raise FBXValidationError("Mesh missing faces/UV0: " + mesh.name)
        for layer in data.uv_layers:
            if len(layer.data) != len(data.loops):
                raise FBXValidationError("UV loop count mismatch: " + mesh.name)
            if any(not math.isfinite(v) for loop in layer.data for v in loop.uv):
                raise FBXValidationError("Nonfinite UVs: " + mesh.name)
        if not rigged:
            continue
        mods = [mod for mod in mesh.modifiers if mod.type == "ARMATURE" and mod.object]
        if len(mods) != 1:
            raise FBXValidationError("Missing source armature: " + mesh.name)
        bones = {b.name for b in mods[0].object.data.bones}
        for vertex in data.vertices:
            weights = []
            for group in vertex.groups:
                if group.group >= len(mesh.vertex_groups):
                    raise FBXValidationError("Invalid vertex group index")
                if not math.isfinite(group.weight) or not 0 <= group.weight <= 1.00001:
                    raise FBXValidationError("Invalid skin weight")
                if group.weight > 0:
                    if mesh.vertex_groups[group.group].name not in bones:
                        raise FBXValidationError("Weight references missing bone")
                    weights.append(group.weight)
            if not weights or abs(sum(weights) - 1) > 0.03:
                raise FBXValidationError("Missing/unnormalized skin weights: " + mesh.name)
    if not embed_textures:
        for mesh in meshes:
            for material in mesh.data.materials:
                if material and material.use_nodes and any(
                        node.type == "TEX_IMAGE" for node in material.node_tree.nodes):
                    raise FBXValidationError("Untextured export retains image nodes")
