"""Lossless original terrain LayerId PNG handoff from glTF to Blender FBX.

glTF extras are not an FBX image slot. Write the exact source bitmap beside
the finished FBX rather than silently dropping its categorical paint data.
This is *not* a reconstructed CryEngine terrain shader.
"""
import json
from pathlib import Path
import struct
import zlib

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
MAX_JSON = 64 * 1024 * 1024
MAX_LAYER_PNG = 128 * 1024 * 1024


class TerrainPaintExportError(RuntimeError):
    pass


def _check(test, message):
    if not test:
        raise TerrainPaintExportError("Original terrain paint: " + message)


def _verified_png(payload, width, height):
    _check(0 < len(payload) <= MAX_LAYER_PNG and payload[:8] == PNG_SIGNATURE,
           "invalid embedded LayerId PNG")
    at = 8
    found_header = False
    found_end = False
    count = 0
    while at < len(payload):
        _check(at + 12 <= len(payload), "truncated PNG chunk")
        size = struct.unpack_from(">I", payload, at)[0]
        kind = payload[at + 4:at + 8]
        end = at + 12 + size
        _check(end <= len(payload) and count < 65536, "oversized PNG chunk")
        source = payload[at + 8:at + 8 + size]
        checksum = struct.unpack_from(">I", payload, at + 8 + size)[0]
        _check(zlib.crc32(kind + source) & 0xffffffff == checksum,
               "PNG chunk CRC mismatch")
        if count == 0:
            _check(kind == b"IHDR" and size == 13, "missing PNG IHDR")
            w, h, bit_depth, color_type = struct.unpack_from(">IIBB", source, 0)
            _check(w == width and h == height and bit_depth == 8
                   and color_type == 6, "unexpected LayerId pixel layout")
            found_header = True
        elif kind == b"IEND":
            _check(size == 0 and end == len(payload), "trailing PNG data")
            found_end = True
            break
        at = end
        count += 1
    _check(found_header and found_end, "incomplete LayerId PNG")


def extract_original_layer_id_png(source):
    """Read only a verified embedded image, or None when no source paint exists.

    The PNG red channel contains the exact categorical LayerId byte for every
    source sample. It is never interpolated, quantized, or converted into color.
    """
    source = Path(source)
    with source.open("rb") as stream:
        head = stream.read(20)
        _check(len(head) == 20 and head[:4] == b"glTF"
               and head[16:20] == b"JSON", "invalid GLB")
        version, total, json_len = struct.unpack_from("<III", head, 4)
        _check(version == 2 and 0 < json_len <= MAX_JSON
               and total == source.stat().st_size, "invalid GLB length")
        document = json.loads(stream.read(json_len))
        marker = document.get("extras", {}).get("original_categorical_terrain_paint")
        if marker is None:
            return None
        _check(document.get("extras", {}).get("source") ==
               "CryEngine Stage 2 level", "untrusted terrain image provenance")
        _check(marker.get("layer_ids_not_surface_type_ids") is True,
               "ambiguous terrain ID semantics")
        width, height = marker.get("width"), marker.get("height")
        _check(type(width) is int and type(height) is int and
               0 < width <= 4096 and 0 < height <= 4096,
               "unsupported bitmap dimensions")
        def indexed(collection, index):
            _check(type(index) is int and
                   0 <= index < len(collection), "invalid glTF paint index")
            return collection[index]
        texture = indexed(document.get("textures", []), marker.get("texture"))
        image = indexed(document.get("images", []), texture.get("source"))
        _check(image.get("mimeType") == "image/png" and
               image.get("name") == "Original CryEngine LayerId (categorical)",
               "unexpected embedded paint resource")
        view = indexed(document.get("bufferViews", []), image.get("bufferView"))
        _check(view.get("buffer", 0) == 0, "invalid binary resource")
        offset, size = view.get("byteOffset", 0), view.get("byteLength")
        _check(type(offset) is int and type(size) is int
               and offset >= 0 and 0 < size <= MAX_LAYER_PNG,
               "invalid embedded bitmap span")
        stream.seek(20 + json_len)
        chunk = stream.read(8)
        _check(len(chunk) == 8 and chunk[4:] == b"BIN\x00",
               "missing GLB binary chunk")
        binary_len = struct.unpack_from("<I", chunk)[0]
        _check(offset + size <= binary_len and
               20 + json_len + 8 + binary_len == total,
               "out-of-range terrain bitmap")
        stream.seek(20 + json_len + 8 + offset)
        payload = stream.read(size)
        _verified_png(payload, width, height)
        return payload


def layer_id_sidecar_path(destination):
    dest = Path(destination)
    return dest.with_name(dest.stem + "_TerrainLayerIDs.png")


def publish_layer_id_sidecar(destination, payload):
    """Never overwrite existing files, including an earlier exported bitmap."""
    target = layer_id_sidecar_path(destination)
    if target.exists():
        if target.is_file() and target.read_bytes() == payload:
            return target
        raise TerrainPaintExportError(
            "Destination terrain paint file already exists: " + str(target))
    # The FBX itself has already passed export/reimport checks before this call.
    # Exclusive creation prevents replacing unrelated user artwork.
    try:
        with target.open("xb") as output:
            output.write(payload)
    except Exception:
        # Never delete an existing file after a failed exclusive open.
        raise
    return target
