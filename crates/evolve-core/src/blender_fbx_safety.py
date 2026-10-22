"""Post-export FBX safety gate. Standard library only: Blender is used solely by scene helpers.

Never modify geometry, UV arrays, bone models, or weighted cluster transforms.
FBX 7.4 (32-bit offsets) and 7.5+ (64-bit offsets) are supported.
"""
import hashlib
import math
import os
import re
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
    embedded_normals = 0
    if not embed_textures:
        embedded_normals = embed_verified_normal_maps(objects, path)

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
    if not embed_textures:
        verify_embedded_normal_media(check_objects, embedded_normals)
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
            "merged_bind_poses": len(poses) - len(pose_groups),
            "embedded_normal_maps": embedded_normals}


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


def _normal_image(material):
    """Find the image actually driving the Principled normal input.

    The glTF importer normally wires Image -> NormalMap -> Principled.
    Unrelated blue image nodes must not be guessed to be normal maps.
    """
    if not material.use_nodes or not material.node_tree:
        return None
    for node in material.node_tree.nodes:
        if node.type != "BSDF_PRINCIPLED":
            continue
        target = node.inputs.get("Normal")
        if target is None or not target.is_linked:
            continue
        normal_node = target.links[0].from_node
        if normal_node.type != "NORMAL_MAP":
            raise FBXValidationError(
                "Unsupported source normal shader graph: " + material.name)
        color = normal_node.inputs.get("Color")
        if color is None or not color.is_linked:
            raise FBXValidationError("Normal-map node has no image: " + material.name)
        image_node = color.links[0].from_node
        if image_node.type != "TEX_IMAGE" or image_node.image is None:
            raise FBXValidationError(
                "Unresolved source normal image: " + material.name)
        return image_node.image
    return None


def _normal_png(image):
    """Do not color-convert packed original DDS->PNG normal maps."""
    packed = image.packed_file
    if packed is not None:
        content = bytes(packed.data)
    else:
        import bpy
        source = Path(bpy.path.abspath(image.filepath))
        if not source.is_file() or source.suffix.lower() != ".png":
            raise FBXValidationError(
                "Original normal PNG not accessible: " + image.name)
        content = source.read_bytes()
    if (not content.startswith(b"\x89PNG\r\n\x1a\n")
            or len(content) > 64 * 1024 * 1024):
        raise FBXValidationError("Source normal map is not a valid-sized PNG")
    return content


def prepare_export_materials(meshes, embed_textures, destination=None):
    """Keep original normal maps inside FBX, even in neutral-white exports.

    Staged PNG files are *temporary input* to the FBX media postprocessor.
    Only the final FBX is published; the staging folder is discarded.
    """
    import bpy
    destination = Path(destination) if destination else None
    saved = {}
    if not embed_textures and destination is None:
        raise FBXValidationError("Embedded normal staging needs an FBX destination")
    for mesh in meshes:
        for material in mesh.data.materials:
            if material is None:
                continue
            if not embed_textures:
                source_normal = _normal_image(material)
                # A CryEngine NoDraw material still occupies its original ID
                # but must remain invisible even in neutral-white exports.
                no_draw = bool(material.get("evolve_source_no_draw", False))
                separate = None
                if source_normal is not None:
                    png = _normal_png(source_normal)
                    digest = hashlib.sha256(png).hexdigest()[:12]
                    filename = destination.stem + "_normal_" + digest + ".png"
                    image_path = destination.parent / "textures" / filename
                    image_path.parent.mkdir(parents=True, exist_ok=True)
                    if image_path.exists() and image_path.read_bytes() != png:
                        raise FBXValidationError("Normal map staging filename collision")
                    if not image_path.exists():
                        image_path.write_bytes(png)
                    separate = saved.get(filename)
                    if separate is None:
                        separate = bpy.data.images.load(str(image_path), check_existing=True)
                        separate.colorspace_settings.name = "Non-Color"
                        saved[filename] = separate
                material.diffuse_color = (1.0, 1.0, 1.0, 0.0 if no_draw else 1.0)
                material.use_nodes = True
                tree = material.node_tree
                tree.nodes.clear()
                output = tree.nodes.new("ShaderNodeOutputMaterial")
                surface = tree.nodes.new("ShaderNodeBsdfPrincipled")
                surface.inputs["Base Color"].default_value = (1., 1., 1., 1.)
                surface.inputs["Alpha"].default_value = 0.0 if no_draw else 1.0
                surface.inputs["Metallic"].default_value = 0.
                surface.inputs["Roughness"].default_value = .5
                tree.links.new(surface.outputs["BSDF"], output.inputs["Surface"])
                if separate is not None:
                    tex = tree.nodes.new("ShaderNodeTexImage")
                    tex.image = separate
                    tex.interpolation = "Linear"
                    normal = tree.nodes.new("ShaderNodeNormalMap")
                    normal.space = "TANGENT"
                    tree.links.new(tex.outputs["Color"], normal.inputs["Color"])
                    tree.links.new(normal.outputs["Normal"], surface.inputs["Normal"])
            elif material.use_nodes:
                for node in material.node_tree.nodes:
                    if node.type != "TEX_IMAGE":
                        continue
                    image = node.image
                    if image is None or not (image.packed_file or
                            (image.filepath and Path(bpy.path.abspath(image.filepath)).is_file())):
                        raise FBXValidationError(
                            "Embedded export has unresolved image: " + material.name)
    return len(saved)


def embed_verified_normal_maps(objects, destination):
    """Embed only authored normal maps, even when other textures are disabled.

    Blender exports neutral materials with their normal map image nodes.
    Embed the source-verified staged PNGs as FBX Video.Content bytes, rather
    than requiring any texture files beside the delivered FBX. Exact image
    hashes and normal-only filenames are checked before rewriting media refs.
    """
    stem = re.escape(destination.stem)
    pattern = re.compile(r"^" + stem + r"_normal_([0-9a-f]{12})\.png$")
    stage = destination.parent / "textures"
    media = {}
    aggregate = 0
    if stage.exists():
        if not stage.is_dir() or stage.is_symlink():
            raise FBXValidationError("Invalid staged normal-map folder")
        for file in stage.iterdir():
            match = pattern.fullmatch(file.name)
            if (not match or not file.is_file() or file.is_symlink()
                    or file.stat().st_size > 64 * 1024 * 1024):
                raise FBXValidationError("Unexpected staged normal-map file")
            aggregate += file.stat().st_size
            if aggregate > 256 * 1024 * 1024 or len(media) >= 256:
                raise FBXValidationError("Too many or oversized embedded normal maps")
            png = file.read_bytes()
            if (not png.startswith(b"\x89PNG\r\n\x1a\n")
                    or hashlib.sha256(png).hexdigest()[:12] != match.group(1)):
                raise FBXValidationError("Staged normal map is not its original PNG")
            media[file.name] = png
    videos, textures = set(), set()
    for node in objects.children:
        if node.name not in (b"Texture", b"Video"):
            continue
        filenames = [child for child in node.children
                     if child.name in (b"Filename", b"FileName", b"RelativeFilename")]
        if not filenames:
            raise FBXValidationError("FBX image has no source normal-map filename")
        names = set()
        for field in filenames:
            path = _text(field).decode("utf-8", errors="replace").replace("\\", "/")
            name = path.rsplit("/", 1)[-1]
            if name not in media:
                raise FBXValidationError("FBX contains an unexpected or missing texture")
            names.add(name)
        if len(names) != 1:
            raise FBXValidationError("FBX normal-map media filenames disagree")
        name = names.pop()
        # Embedded media should not refer to an absolute temporary directory.
        for field in filenames:
            label = name.encode("utf-8")
            field.props = b"S" + struct.pack("<I", len(label)) + label
            field.count = 1
        if node.name == b"Texture":
            textures.add(name)
            continue
        # Blender can serialize one Video object per material instance even
        # when multiple materials share the same underlying normal image.
        # Embed and validate every Video, but count distinct source images.
        videos.add(name)
        content_nodes = node.children_named(b"Content")
        if len(content_nodes) > 1:
            raise FBXValidationError("Ambiguous embedded FBX media Content")
        png = media[name]
        encoded = b"R" + struct.pack("<I", len(png)) + png
        if content_nodes:
            content_nodes[0].props = encoded
            content_nodes[0].count = 1
        else:
            node.children.append(Node(b"Content", encoded, 1))
            node.terminator = True
    if videos != set(media) or textures != set(media):
        raise FBXValidationError("FBX normal map lacks an image or texture binding")
    return len(media)


def verify_embedded_normal_media(objects, count):
    """Check actual serialized FBX Video.Content bytes, not disk links."""
    videos = objects.children_named(b"Video")
    found = {}
    for video in videos:
        contents = video.children_named(b"Content")
        if len(contents) != 1:
            raise FBXValidationError("FBX normal map is not embedded")
        props = _properties(contents[0])
        if (len(props) != 1 or not isinstance(props[0], bytes)
                or not props[0].startswith(b"\x89PNG\r\n\x1a\n")):
            raise FBXValidationError("FBX lacks valid embedded normal-map PNG bytes")
        names = [child for child in video.children
                 if child.name in (b"Filename", b"FileName", b"RelativeFilename")]
        if not names:
            raise FBXValidationError("Embedded normal map lost its filename")
        filename = _text(names[0]).decode("utf-8", errors="replace")
        if not filename.endswith(".png") or "/" in filename or "\\" in filename:
            raise FBXValidationError("Embedded normal map has unsafe media name")
        digest = filename.rsplit("_normal_", 1)[-1].removesuffix(".png")
        if len(digest) != 12 or hashlib.sha256(props[0]).hexdigest()[:12] != digest:
            raise FBXValidationError("Embedded normal-map payload hash mismatch")
        if filename in found and found[filename] != props[0]:
            raise FBXValidationError("Duplicate normal images have inconsistent embedded bytes")
        found[filename] = props[0]
    if len(found) != count:
        raise FBXValidationError("Embedded normal-map identity count changed")



def normal_image_names(meshes):
    """Source/round-trip normal-map identity, independent of mesh splits."""
    names = set()
    for mesh in meshes:
        for material in mesh.data.materials:
            if material is None:
                continue
            image = _normal_image(material)
            if image is not None:
                names.add(Path(image.filepath.replace("\\", "/")).name)
    return names


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
                if not material or not material.use_nodes:
                    continue
                allowed = _normal_image(material)
                for node in material.node_tree.nodes:
                    if node.type == "TEX_IMAGE" and node.image != allowed:
                        raise FBXValidationError(
                            "Unembedded FBX retains a non-normal image node")
                if allowed is not None:
                    import bpy
                    if not Path(bpy.path.abspath(allowed.filepath)).is_file():
                        raise FBXValidationError("Staged embedded normal-map input missing")
