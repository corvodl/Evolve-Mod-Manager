"""Source-exact categorical terrain boundary atlas for Blender editing.

Each pixel reports original LayerId (blue), neighbor-ID changes in right/down
direction (red/green), and boundary presence (alpha). No native CryEngine
blend width, texture interpolation, or shader weights are inferred.
"""
from pathlib import Path
import struct
import zlib
from blender_level_paint import (
    _verified_png, _check, TerrainPaintExportError, PNG_SIGNATURE
)


def _chunks(kind, data):
    return (struct.pack(">I", len(data)) + kind + data +
            struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff))


def _red_bytes(payload):
    if len(payload) < 33 or payload[:8] != PNG_SIGNATURE:
        raise TerrainPaintExportError("Missing original categorical PNG")
    width, height = struct.unpack_from(">II", payload, 16)
    _check(0 < width <= 4096 and 0 < height <= 4096, "invalid source dimensions")
    _verified_png(payload, width, height)
    compressed = bytearray()
    offset = 8
    while offset < len(payload):
        length = struct.unpack_from(">I", payload, offset)[0]
        typ = payload[offset+4:offset+8]
        if typ == b"IDAT":
            _check(len(compressed) + length <= 128*1024*1024,
                   "oversized source PNG stream")
            compressed.extend(payload[offset+8:offset+8+length])
        offset += length + 12
        if typ == b"IEND":
            break
    row_len = width * 4
    expected = (row_len + 1) * height
    inflate = zlib.decompressobj()
    raw = inflate.decompress(compressed, expected + 1)
    _check(len(raw) == expected and inflate.eof
           and not inflate.unconsumed_tail and not inflate.unused_data,
           "truncated or oversized categorical pixels")
    ids = bytearray(width * height)
    prior = bytearray(row_len)
    at = 0
    for y in range(height):
        filt = raw[at]
        at += 1
        row = bytearray(row_len)
        for i, original in enumerate(raw[at:at+row_len]):
            left = row[i-4] if i >= 4 else 0
            up = prior[i]
            up_left = prior[i-4] if i >= 4 else 0
            if filt == 0:
                predictor = 0
            elif filt == 1:
                predictor = left
            elif filt == 2:
                predictor = up
            elif filt == 3:
                predictor = (left + up)//2
            elif filt == 4:
                p = left + up - up_left
                distances = (abs(p-left), abs(p-up), abs(p-up_left))
                predictor = (left, up, up_left)[distances.index(min(distances))]
            else:
                raise TerrainPaintExportError("Invalid PNG scanline filter")
            row[i] = (original + predictor) & 255
        for x in range(width):
            ids[y*width+x] = row[4*x]
        prior = row
        at += row_len
    return width, height, ids


def source_transition_boundaries(payload):
    """Return an exact edge-location PNG and original neighboring-ID edge count.

    R=255 means this sample differs from the *right* source pixel;
    G=255 means this sample differs from the *down* source pixel;
    B=original LayerId, A=255 at an authentic categorical boundary.
    All other alpha pixels are zero. This is NOT a native blend mask.
    """
    width, height, ids = _red_bytes(payload)
    rows = bytearray()
    count = 0
    for y in range(height):
        rows.append(0)  # PNG scanline filter None
        for x in range(width):
            index=y*width+x
            value=ids[index]
            right=int(x+1<width and ids[index+1]!=value)
            down=int(y+1<height and ids[index+width]!=value)
            rows.extend((right*255,down*255,value,int(right or down)*255))
            count+=right+down
    ihdr=struct.pack(">IIBBBBB",width,height,8,6,0,0,0)
    png=(PNG_SIGNATURE + _chunks(b"IHDR",ihdr) +
         _chunks(b"IDAT",zlib.compress(rows,6)) + _chunks(b"IEND",b""))
    _verified_png(png,width,height)
    return png,count


def transition_sidecar_path(destination):
    dest=Path(destination)
    return dest.with_name(dest.stem+"_TerrainBoundaries.png")


def publish_transition_sidecar(destination,payload):
    target=transition_sidecar_path(destination)
    if target.exists():
        if target.is_file() and target.read_bytes()==payload:
            return target
        raise TerrainPaintExportError("Original terrain boundary file already exists: "+
                                      str(target))
    with target.open("xb") as output:
        output.write(payload)
    return target
