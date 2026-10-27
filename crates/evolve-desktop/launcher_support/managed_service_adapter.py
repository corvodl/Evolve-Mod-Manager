"""Read-only .NET metadata inspection for a version-independent Stage2 service adapter.

This is NOT a generic IL patcher: only Stage2Smoke.Run's precisely verified
no-launch exit can be replaced, and only in a disposable client shadow.
Any changed semantics or missing metadata fails closed.
"""
from __future__ import annotations


class UnsupportedClient(RuntimeError):
    pass


def _read(data, at, length):
    if at < 0 or length < 0 or at + length > len(data):
        raise UnsupportedClient("Truncated managed client metadata")
    return data[at:at + length]


def _num(data, at, length):
    return int.from_bytes(_read(data, at, length), "little")


def _compressed(data, at):
    lead = _num(data, at, 1)
    if lead < 0x80:
        return lead, 1
    if lead < 0xC0:
        return ((lead & 0x3F) << 8) | _num(data, at + 1, 1), 2
    if lead < 0xE0:
        return ((lead & 0x1F) << 24) | (_num(data, at + 1, 1) << 16) | _num(data, at + 2, 2), 4
    raise UnsupportedClient("Malformed metadata length")


class _ManagedMetadata:
    def __init__(self, data):
        self.data = data
        if _read(data, 0, 2) != b"MZ":
            raise UnsupportedClient("Client is not a Windows DLL")
        pe = _num(data, 0x3C, 4)
        if _read(data, pe, 4) != b"PE\x00\x00":
            raise UnsupportedClient("Missing Windows PE header")
        opt = pe + 24
        magic = _num(data, opt, 2)
        if magic not in (0x10B, 0x20B):
            raise UnsupportedClient("Unsupported PE layout")
        section_count = _num(data, pe + 6, 2)
        if not 1 <= section_count <= 32:
            raise UnsupportedClient("Unexpected section count")
        section_start = opt + _num(data, pe + 20, 2)
        self.sections = []
        for n in range(section_count):
            pos = section_start + n * 40
            self.sections.append((
                _num(data, pos + 12, 4),
                max(_num(data, pos + 8, 4), _num(data, pos + 16, 4)),
                _num(data, pos + 20, 4),
                _num(data, pos + 16, 4),
            ))
        directory = opt + (112 if magic == 0x20B else 96)
        cor_rva = _num(data, directory + 14 * 8, 4)
        if not cor_rva:
            raise UnsupportedClient("Expected a managed .NET assembly")
        cor = self.map(cor_rva)
        root = self.map(_num(data, cor + 8, 4))
        if _read(data, root, 4) != b"BSJB":
            raise UnsupportedClient("Missing .NET metadata header")
        version_end = (root + 16 + _num(data, root + 12, 4) + 3) & ~3
        stream_count = _num(data, version_end + 2, 2)
        if not 1 <= stream_count <= 16:
            raise UnsupportedClient("Unexpected managed metadata streams")
        pos = version_end + 4
        streams = {}
        for _ in range(stream_count):
            off = _num(data, pos, 4)
            length = _num(data, pos + 4, 4)
            start = pos + 8
            end = data.find(b"\x00", start, min(len(data), start + 33))
            if end < 0:
                raise UnsupportedClient("Invalid metadata stream name")
            name = _read(data, start, end - start).decode("ascii")
            _read(data, root + off, length)
            streams[name] = root + off, length
            pos = (end + 4) & ~3
        try:
            self.strings = streams["#Strings"][0]
            self.blobs = streams["#Blob"][0]
            self.us, self.us_length = streams["#US"]
            table_header = streams["#~"][0]
        except KeyError as error:
            raise UnsupportedClient("Required .NET metadata stream is absent") from error
        self.heap = _num(data, table_header + 6, 1)
        mask = _num(data, table_header + 8, 8)
        pos = table_header + 24
        self.counts = {}
        for index in range(64):
            if (mask >> index) & 1:
                self.counts[index] = _num(data, pos, 4)
                pos += 4
        si, ti, ci = self.si, self.ti, self.ci
        widths = {
            0: 2 + si(0) + 3 * si(1),
            1: ci((0, 26, 35, 1), 2) + 2 * si(0),
            2: 4 + 2 * si(0) + ci((2, 1, 27), 2) + ti(4) + ti(6),
            3: ti(4),
            4: 2 + si(0) + si(2),
            5: ti(6),
            6: 8 + si(0) + si(2) + ti(8),
            7: ti(8),
            8: 4 + si(0),
            9: ti(2) + ci((2, 1, 27), 2),
            10: ci((2, 1, 26, 6, 27), 3) + si(0) + si(2),
        }
        self.widths = widths
        self.starts = {}
        for index in range(11):
            if index in self.counts:
                self.starts[index] = pos
                pos += self.counts[index] * widths[index]
        for index in (1, 2, 6, 10):
            if index not in self.counts:
                raise UnsupportedClient("Required .NET metadata table is absent")

    def map(self, rva):
        for start, span, raw, raw_length in self.sections:
            if start <= rva < start + min(span, raw_length):
                position = raw + rva - start
                _read(self.data, position, 1)
                return position
        raise UnsupportedClient("Unmapped method RVA")

    def si(self, heap):
        return 4 if (self.heap >> heap) & 1 else 2

    def ti(self, table):
        return 4 if self.counts.get(table, 0) >= 65536 else 2

    def ci(self, tables, tag_bits):
        limit = 1 << (16 - tag_bits)
        return 4 if max(self.counts.get(table, 0) for table in tables) >= limit else 2

    def row(self, table, index):
        if not 1 <= index <= self.counts.get(table, 0):
            raise UnsupportedClient("Invalid managed metadata row")
        return self.starts[table] + (index - 1) * self.widths[table]

    def string(self, index):
        at = self.strings + index
        end = self.data.find(b"\x00", at, min(len(self.data), at + 1024))
        if end < 0:
            raise UnsupportedClient("Malformed managed metadata string")
        return _read(self.data, at, end - at).decode("utf-8")

    def blob(self, index):
        at = self.blobs + index
        size, prefix = _compressed(self.data, at)
        return _read(self.data, at + prefix, size)

    def type_name(self, table, index):
        at = self.row(table, index)
        if table == 1:
            at += self.ci((0, 26, 35, 1), 2)
        elif table == 2:
            at += 4
        else:
            raise UnsupportedClient("Unexpected metadata type table")
        return (self.string(_num(self.data, at, self.si(0))),
                self.string(_num(self.data, at + self.si(0), self.si(0))))

    def method(self, index):
        at = self.row(6, index)
        return _num(self.data, at, 4), self.string(_num(self.data, at + 8, self.si(0)))

    def stage2_run(self):
        matches = []
        for i in range(1, self.counts[2] + 1):
            if self.type_name(2, i) != ("Stage2Smoke", "ModdedEvolveLauncher.Services"):
                continue
            at = self.row(2, i)
            start_offset = at + 4 + 2 * self.si(0) + self.ci((2, 1, 27), 2) + self.ti(4)
            start = _num(self.data, start_offset, self.ti(6))
            stop = self.counts[6] + 1
            if i < self.counts[2]:
                nxt = self.row(2, i + 1)
                stop = _num(self.data, nxt + 4 + 2 * self.si(0)
                            + self.ci((2, 1, 27), 2) + self.ti(4), self.ti(6))
            methods = {index: self.method(index) for index in range(start, stop)}
            runs = [rva for rva, name in methods.values() if name == "Run" and rva]
            loggers = [index for index, (_, name) in methods.items()
                       if name.startswith("<Run>g__Say|")]
            if len(runs) != 1 or len(loggers) != 1:
                raise UnsupportedClient("Stage2Smoke.Run control flow changed")
            matches.append((runs[0], loggers[0]))
        if len(matches) != 1:
            raise UnsupportedClient("Cannot uniquely resolve Stage2Smoke.Run")
        return matches[0]

    def sleep_token(self):
        matches = []
        parent_width = self.ci((2, 1, 26, 6, 27), 3)
        for index in range(1, self.counts[10] + 1):
            at = self.row(10, index)
            parent = _num(self.data, at, parent_width)
            if (parent & 7) != 1:
                continue
            if self.type_name(1, parent >> 3) != ("Thread", "System.Threading"):
                continue
            name = self.string(_num(self.data, at + parent_width, self.si(0)))
            signature = self.blob(_num(self.data, at + parent_width + self.si(0), self.si(2)))
            if name == "Sleep" and signature == b"\x00\x01\x01\x08":
                matches.append(index)
        if len(matches) != 1:
            raise UnsupportedClient("Cannot uniquely resolve Thread.Sleep(Int32)")
        return 0x0A000000 | matches[0]

    def stop_token(self):
        position, end = self.us + 1, self.us + self.us_length
        matches = []
        while position < end:
            size, prefix = _compressed(self.data, position)
            blob = _read(self.data, position + prefix, size)
            if size >= 3 and size % 2 == 1:
                message = blob[:-1].decode("utf-16-le", errors="replace")
                if message == "--no-launch: stopping here":
                    matches.append(0x70000000 | (position - self.us))
            position += prefix + size
        if len(matches) != 1:
            raise UnsupportedClient("Missing unique no-launch marker")
        return matches[0]

    def code(self, rva):
        at = self.map(rva)
        first = _num(self.data, at, 1)
        if first & 3 == 2:
            begin, size = at + 1, first >> 2
        elif first & 3 == 3:
            head = _num(self.data, at, 2)
            header_size = 4 * (head >> 12)
            if header_size < 12:
                raise UnsupportedClient("Invalid method header")
            begin, size = at + header_size, _num(self.data, at + 4, 4)
        else:
            raise UnsupportedClient("Unsupported method body format")
        if not 32 <= size <= 1024 * 1024:
            raise UnsupportedClient("Unexpected Stage2Smoke.Run size")
        _read(self.data, begin, size)
        return begin, size


def resolve(data):
    """Identify the one compatible no-launch IL block; no bytes are modified."""
    try:
        meta = _ManagedMetadata(data)
        run_rva, logger = meta.stage2_run()
        stop_token = meta.stop_token()
        sleep_token = meta.sleep_token()
        begin, size = meta.code(run_rva)
        # Log the exact '--no-launch: stopping here' marker via Run's own
        # Say helper, store false, then leave. Disallow wildcard matching.
        prefix = (b"\x72" + stop_token.to_bytes(4, "little")
                  + b"\x28" + (0x06000000 | logger).to_bytes(4, "little")
                  + b"\x16\x13\x14\xdd")
        method_body = _read(data, begin, size)
        at = method_body.find(prefix)
        if at < 0 or method_body.find(prefix, at + 1) >= 0:
            raise UnsupportedClient("Stage2Smoke.Run no-launch exit changed")
        offset = begin + at
        original = _read(data, offset, 18)
        target = at + 18 + int.from_bytes(original[14:18], "little", signed=True)
        if not 0 <= target < size:
            raise UnsupportedClient("Stage2Smoke.Run branch target changed")
        replacement = (b"\x20\xe8\x03\x00\x00\x28"
                       + sleep_token.to_bytes(4, "little") + b"\x2b\xf4" + bytes(6))
        return offset, original, replacement
    except (UnicodeError, KeyError, ValueError, IndexError) as error:
        raise UnsupportedClient("Cannot verify this client's managed IL layout") from error
