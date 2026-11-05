"""Read-only authentication-scoped audit of Evolve compiled terrain surfaces.

The caller must authenticate/extract original game PAKs before passing any
terrain bytes. This does NOT recover pPass.fAlpha or native blend masks.
Only the observed v25 container with v7, 33x33 finest terrain sectors is
supported; unsupported layouts fail rather than interpolating invented data.
"""
from dataclasses import dataclass
import math
import struct

MAX_TERRAIN_BYTES = 80 * 1024 * 1024
_CHUNK = struct.Struct("<4BI4i2f")
_NODE = struct.Struct("<hh6f2f2i")
_SIDE = 33
_SAMPLES = _SIDE * _SIDE
# Source node: 44-byte header, 33*33 u16 packed samples, 0xDEDE and 20
# native metadata bytes, then the local SurfaceTypeID palette.
_PALETTE_AT = _NODE.size + _SAMPLES * 2 + 22


class CompiledTerrainSourceError(ValueError):
    pass


@dataclass(frozen=True)
class TerrainSurfaceSector:
    x: int
    y: int
    palette: bytes
    packed: bytes

    def source_surface_type_at(self, dx, dy):
        """Discrete source SurfaceTypeID; None for a reserved palette index.

        The bottom nibble of a packed u16 chooses a local palette entry.
        It is NOT a continuous native paint alpha.
        """
        if not (0 <= dx < _SIDE and 0 <= dy < _SIDE):
            raise IndexError("terrain vertex outside source sector")
        word = struct.unpack_from("<H", self.packed, (dx * _SIDE + dy) * 2)[0]
        index = word & 0xF
        return self.palette[index] if index < len(self.palette) else None


@dataclass(frozen=True)
class CompiledTerrainSurfaceData:
    width: int
    sector_meters: int
    sectors: dict

    def source_surface_types(self):
        """Return categorical source grid, 255 where unavailable/unknown.

        This is a surface classification diagnostic, not a splat or alpha map.
        Never pass it to GetLayerBlendingValue as the original pPass.fAlpha.
        """
        out = bytearray([255]) * (self.width * self.width)
        for (x, y), sector in self.sectors.items():
            for dx in range(self.sector_meters):
                for dy in range(self.sector_meters):
                    value = sector.source_surface_type_at(dx, dy)
                    if value is not None:
                        out[(x + dx) * self.width + y + dy] = value
        return bytes(out)


def parse_compiled_terrain_surface_data(data):
    """Recover verified-format v7 finest-sector surface classifications.

    Only pass data extracted from an authenticated same-build CryPAK. Scanning
    unknown node sections requires strict dimensions, metadata and sentinels,
    and does not prove the existence of native per-fragment paint weights.
    """
    if not isinstance(data, bytes) or not 32 <= len(data) <= MAX_TERRAIN_BYTES:
        raise CompiledTerrainSourceError("expected bounded compiled terrain bytes")
    (ver, reserved, flags, flags2, length, width, unit, sector_size,
     table_size, zratio, ocean) = _CHUNK.unpack_from(data)
    if ((ver, reserved, flags, flags2) != (25, 0, 2, 1)
            or length != len(data)):
        raise CompiledTerrainSourceError("unsupported Evolve terrain container")
    if (unit != 1 or sector_size != 32 or table_size <= 0
            or width != sector_size * table_size or width > 4096
            or not math.isfinite(zratio) or zratio <= 0
            or not math.isfinite(ocean)):
        raise CompiledTerrainSourceError("unsupported source dimensions/scales")
    found = {}
    min_node_bytes = _NODE.size + _SAMPLES * 2 + 22
    for at in range(_CHUNK.size, len(data) - min_node_bytes, 4):
        if data[at:at + 4] != b"\x07\x00\x00\x00":
            continue
        (version, holes, x, y, z, x2, y2, z2, offset,
         scale, count_on_side, palette_count) = _NODE.unpack_from(data, at)
        if (version != 7 or holes not in (0, 1) or count_on_side != _SIDE
                or not 1 <= palette_count <= 16
                or x2 - x != sector_size or y2 - y != sector_size
                or any(not math.isfinite(v) for v in
                       (x, y, z, x2, y2, z2, offset, scale))
                or x < 0 or y < 0 or x >= width or y >= width
                or z2 < z or x % sector_size or y % sector_size or scale < 0):
            continue
        samples_at = at + _NODE.size
        samples_end = samples_at + _SAMPLES * 2
        if data[samples_end:samples_end + 2] != b"\xde\xde":
            continue
        palette = data[at + _PALETTE_AT:at + _PALETTE_AT + palette_count]
        if len(palette) != palette_count:
            raise CompiledTerrainSourceError("truncated original surface palette")
        key = (int(x), int(y))
        sector = TerrainSurfaceSector(*key, palette, data[samples_at:samples_end])
        if key in found and found[key] != sector:
            raise CompiledTerrainSourceError("conflicting duplicate finest sector")
        found[key] = sector
    if not found or len(found) > table_size * table_size:
        raise CompiledTerrainSourceError("no supported finest-sector surfaces")
    return CompiledTerrainSurfaceData(width, sector_size, found)


def original_heightmap_compiled_section(source):
    """Read the original source Heightmap.dat's sized compiled terrain tail."""
    if (not isinstance(source, bytes)
            or not 64 <= len(source) <= MAX_TERRAIN_BYTES or source[0] != 255):
        raise CompiledTerrainSourceError("invalid original Heightmap.dat")
    marker = b"TerrainCompiledData"
    at = source.rfind(marker)
    if at < 0 or at + len(marker) + 12 > len(source):
        raise CompiledTerrainSourceError("compiled terrain section missing")
    size_a, size_b, codec = struct.unpack_from("<III", source, at + len(marker))
    start = at + len(marker) + 12
    if (codec != 0 or size_a != size_b or size_a < 32
            or start + size_a != len(source)):
        raise CompiledTerrainSourceError("unexpected compiled terrain length/codec")
    return source[start:]


def compare_compiled_surface_sectors(source, candidate):
    """Match exact sector palettes and vertices across two terrain revisions.

    Other container sections may differ. Identical map names or signatures
    alone are not sufficient evidence for combining cross-archive terrain.
    """
    left = parse_compiled_terrain_surface_data(source)
    right = parse_compiled_terrain_surface_data(candidate)
    if left.width != right.width or left.sector_meters != right.sector_meters:
        raise CompiledTerrainSourceError("source terrain grid differs")
    if left.sectors.keys() != right.sectors.keys():
        raise CompiledTerrainSourceError("compiled terrain sector coverage differs")
    mismatch = sum(left.sectors[key] != right.sectors[key] for key in left.sectors)
    if mismatch:
        raise CompiledTerrainSourceError(
            f"{mismatch} compiled terrain surface sectors differ")
    return len(left.sectors)
