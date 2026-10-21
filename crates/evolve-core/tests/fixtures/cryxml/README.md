# Synthetic CryXmlB fixtures

Each `.bin` contains the adjacent `.xml` document encoded with the reference
manager's `encode_cryxml` implementation (CryEngine 3's 44-byte header,
28-byte nodes, attribute pairs, child indices and UTF-8 string table).
The documents were authored for these tests and contain no game data.

The Rust decoder does not generate these fixtures, so the tests cross-check
the binary format independently of its implementation.
