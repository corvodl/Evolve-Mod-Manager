"""Encode a test XML member with the table layout observed in shipped Stage 2 CryXmlB.

The general-purpose Rust encoder produces a different table layout; the game
reported its first dome probe as corrupt. This experimental encoder is kept
separate until the game accepts it. It never modifies the source document.
"""

import argparse
import struct
from pathlib import Path
import xml.etree.ElementTree as ET


def encode(xml: bytes) -> bytes:
    root = ET.fromstring(xml)
    nodes = []

    def walk(node, parent):
        index = len(nodes)
        nodes.append((node, parent, []))
        if node.tail and node.tail.strip():
            raise ValueError("Mixed content is unsupported")
        for child in node:
            nodes[index][2].append(walk(child, index))
        return index

    walk(root, -1)
    strings = bytearray()
    offsets = {}

    def string(value):
        if value not in offsets:
            offsets[value] = len(strings)
            strings.extend(value.encode("utf-8") + b"\0")
        return offsets[value]

    records = bytearray()
    children = bytearray()
    attributes = bytearray()
    for node, parent, child_indices in nodes:
        if len(node.attrib) > 65535 or len(child_indices) > 65535:
            raise ValueError("CryXmlB node count limit")
        name = string(node.tag)
        content = string(node.text or "")
        first_attribute = len(attributes) // 8
        first_child = len(children) // 4
        for key, value in node.attrib.items():
            attributes.extend(struct.pack("<II", string(key), string(value)))
        for child in child_indices:
            children.extend(struct.pack("<I", child))
        records.extend(struct.pack("<IIHHIIII", name, content, len(node.attrib),
                                   len(child_indices), parent & 0xFFFFFFFF,
                                   first_attribute, first_child, 0))

    # Stock Stage 2 files place children before attributes, then strings.
    children_at = 44 + len(records)
    attributes_at = children_at + len(children)
    strings_at = attributes_at + len(attributes)
    size = strings_at + len(strings)
    if size > 16 * 1024 * 1024:
        raise ValueError("CryXmlB output limit")
    header = b"CryXmlB\0" + struct.pack("<9I", size, 44, len(nodes),
                                          attributes_at, len(attributes) // 8,
                                          children_at, len(children) // 4,
                                          strings_at, len(strings))
    return header + records + children + attributes + strings


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("refusing to overwrite existing output")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(encode(args.source.read_bytes()))
    print(f"Wrote native-layout CryXmlB probe: {args.output}")
